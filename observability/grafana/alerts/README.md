# Grafana Alerts Configuration

This directory contains Grafana alerting configuration for Madara and Orchestrator services.

## Directory Structure

```
alerts/
├── README.md                      # This file
├── contact_points.yaml            # WHERE alerts go (Slack, Email, etc.)
├── notification_policies.yaml     # HOW alerts are routed
├── Madara/
│   └── madara_alerts.yaml         # Madara node alert rules
└── Orchestrator/
    ├── orchestrator_alerts.yaml   # Orchestrator alert rules
    ├── advanced_alerts.yaml       # Advanced resource/data integrity alerts
    └── notification_channels.yaml # (Legacy - use contact_points.yaml instead)
```

## Setup for Kubernetes

### Step 1: Create Slack Webhook

1. Go to [Slack API](https://api.slack.com/messaging/webhooks)
2. Create an incoming webhook for your alerts channel
3. Copy the webhook URL

### Step 2: Create Kubernetes Secret

```bash
kubectl create secret generic grafana-alerting-secrets \
  --from-literal=SLACK_WEBHOOK_URL='https://hooks.slack.com/services/xxx/yyy/zzz' \
  -n <grafana-namespace>
```

### Step 3: Create ConfigMaps

```bash
# From the madara repository root
cd observability/grafana/alerts

# Create ConfigMap for contact points and policies
kubectl create configmap grafana-alerting-config \
  --from-file=contact_points.yaml \
  --from-file=notification_policies.yaml \
  -n <grafana-namespace>

# Create ConfigMap for Madara alerts
kubectl create configmap madara-alerts \
  --from-file=Madara/ \
  -n <grafana-namespace>

# Create ConfigMap for Orchestrator alerts
kubectl create configmap orchestrator-alerts \
  --from-file=Orchestrator/orchestrator_alerts.yaml \
  --from-file=Orchestrator/advanced_alerts.yaml \
  -n <grafana-namespace>
```

### Step 4: Mount in Grafana Deployment

Add these volumes and mounts to your Grafana deployment:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: grafana
spec:
  template:
    spec:
      containers:
        - name: grafana
          # Inject the Slack webhook URL from secret
          env:
            - name: SLACK_WEBHOOK_URL
              valueFrom:
                secretKeyRef:
                  name: grafana-alerting-secrets
                  key: SLACK_WEBHOOK_URL
          volumeMounts:
            # Alerting configuration (contact points + policies)
            - name: alerting-config
              mountPath: /etc/grafana/provisioning/alerting
            # Madara alert rules
            - name: madara-alerts
              mountPath: /etc/grafana/provisioning/alerting/Madara
            # Orchestrator alert rules
            - name: orchestrator-alerts
              mountPath: /etc/grafana/provisioning/alerting/Orchestrator
      volumes:
        - name: alerting-config
          configMap:
            name: grafana-alerting-config
        - name: madara-alerts
          configMap:
            name: madara-alerts
        - name: orchestrator-alerts
          configMap:
            name: orchestrator-alerts
```

### Step 5: Update Datasource UID

The alert rules reference a datasource UID (`PBFA97CFB590B2093`). You need to update this to match your Prometheus datasource:

1. Go to Grafana UI → Configuration → Data Sources → Prometheus
2. Note the UID from the URL (e.g., `/datasources/edit/abc123`)
3. Replace `PBFA97CFB590B2093` in all alert YAML files:

```bash
# Find and replace datasource UID
find . -name "*.yaml" -exec sed -i 's/PBFA97CFB590B2093/YOUR_DATASOURCE_UID/g' {} \;
```

## Alert Summary

### Madara Alerts (15 alerts, all warnings)

| Category | Alert | Trigger |
|----------|-------|---------|
| Cairo Native | Compilation Failures | Any compilation failures |
| Cairo Native | Compilation Timeouts | Any compilation timeouts |
| Cairo Native | VM Fallback Rate | > 0.1 fallbacks/sec |
| Database | Size Warning | > 100GB |
| Database | Writes Stopped | RocksDB write stall |
| Database | Compaction Backlog | > 4GB pending |
| Database | L0 Files High | > 15 L0 files |
| Database | Immutable Memtables | > 3 memtables |
| Database | Memtable Memory | > 2GB |
| RPC | P50 Latency High | > 1 second |
| RPC | P99 Latency High | > 5 seconds |
| RPC | Slow Method | P50 > 2s per method |
| RPC | Block Production Slow | > 10 seconds |
| RPC | TX Execution Slow | P50 > 500ms |
| RPC | Error Rate High | > 5% errors |

### Orchestrator Alerts

See `Orchestrator/README.md` for details.

## Customization

### Adjust Thresholds

Edit the threshold values in the alert YAML files. Look for `evaluator.params`:

```yaml
evaluator:
  params:
    - 1000  # Change this value (e.g., 1000ms = 1 second)
  type: gt
```

### Add More Channels

Edit `contact_points.yaml` to add Email, PagerDuty, etc.:

```yaml
# Add Email
- orgId: 1
  name: email-team
  receivers:
    - uid: email-receiver
      type: email
      settings:
        addresses: team@example.com;oncall@example.com

# Add PagerDuty
- orgId: 1
  name: pagerduty-oncall
  receivers:
    - uid: pagerduty-receiver
      type: pagerduty
      settings:
        integrationKey: ${PAGERDUTY_KEY}
        severity: critical
```

### Route by Severity

Edit `notification_policies.yaml` to route critical alerts differently:

```yaml
routes:
  - match:
      severity: critical
    receiver: pagerduty-oncall
    group_wait: 0s
    repeat_interval: 30m

  - match:
      severity: warning
    receiver: slack-alerts
    group_wait: 30s
    repeat_interval: 4h
```

## Troubleshooting

### Alerts not firing

1. Check if Prometheus datasource is configured correctly
2. Verify the datasource UID matches in alert rules
3. Check Grafana logs: `kubectl logs -f <grafana-pod>`

### Notifications not received

1. Verify Slack webhook URL is correct
2. Check if the secret is mounted: `kubectl exec <grafana-pod> -- env | grep SLACK`
3. Test webhook manually: `curl -X POST -H 'Content-type: application/json' --data '{"text":"Test"}' $SLACK_WEBHOOK_URL`

### View alert state

Go to Grafana UI → Alerting → Alert rules to see current state of all alerts.
