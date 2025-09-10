# Madara Orchestrator Alert Configuration Guide

## Overview

This directory contains comprehensive alert rules and notification configurations for monitoring the Madara Orchestrator system.
The alerts are designed to detect issues early and provide actionable insights for maintaining system health.

## Alert Categories

### 1. System Health Alerts

- **Orchestrator Service Down**: Detects when the main orchestrator service is unresponsive
- **Health Check Failures**: Monitors the `/health` endpoint availability

### 2. Job Processing Pipeline Alerts

- **Block Processing Lag**: Detects when one job type is significantly behind others
- **High Job Failure Rate**: Alerts when job failure rate exceeds thresholds
- **No Block Progress**: Detects system stalls when no blocks are being processed

### 3. Performance & SLA Alerts

- **Job Response Time**: Monitors processing time for each job type
- **Database Response Time**: Tracks MongoDB performance

### 4. External Dependencies

- **Prover Service Failures**: Detects issues with proof generation services
- **DA Layer Issues**: Monitors data availability layer connectivity
- **Settlement Layer Issues**: Tracks L1/settlement layer interactions

### 5. Queue & Worker Health

- **Queue Depth Monitoring**: Alerts on message backlog
- **Worker Restart Detection**: Tracks unexpected worker failures

### 6. Resource Management

- **Batch Size Monitoring**: Prevents exceeding blob size limits
- **Graceful Shutdown Monitoring**: Ensures clean shutdowns

### 7. Data Integrity

- **Jobs Stuck in Locked State**: Detects potential deadlocks
- **Verification Failure Patterns**: Identifies systematic issues
- **Consecutive Failures**: Tracks repeated failures requiring intervention

## Setup Instructions

### 1. Import Alert Rules

```bash
# Using Grafana API
curl -X POST -H "Content-Type: application/json" \
  -H "Authorization: Bearer $GRAFANA_API_KEY" \
  -d @orchestrator_alerts.yaml \
  http://localhost:3000/api/v1/provisioning/alert-rules

# Import advanced alerts
curl -X POST -H "Content-Type: application/json" \
  -H "Authorization: Bearer $GRAFANA_API_KEY" \
  -d @advanced_alerts.yaml \
  http://localhost:3000/api/v1/provisioning/alert-rules
```

### 2. Configure Notification Channels

First, set the required environment variables:

```bash
export SLACK_WEBHOOK_URL_CRITICAL="https://hooks.slack.com/services/YOUR/CRITICAL/WEBHOOK"
export SLACK_WEBHOOK_URL_WARNING="https://hooks.slack.com/services/YOUR/WARNING/WEBHOOK"
export PAGERDUTY_INTEGRATION_KEY="your-pagerduty-key"
export CUSTOM_WEBHOOK_URL="https://your-webhook-endpoint.com"
export WEBHOOK_PASSWORD="your-webhook-password"
```

Then import the notification channels:

```bash
curl -X POST -H "Content-Type: application/json" \
  -H "Authorization: Bearer $GRAFANA_API_KEY" \
  -d @notification_channels.yaml \
  http://localhost:3000/api/v1/provisioning/contact-points
```

### 3. Using Grafana UI

Alternatively, you can import via Grafana UI:

1. Navigate to **Alerting** → **Alert rules**
2. Click **Import alert rules**
3. Copy the content from `orchestrator_alerts.yaml`
4. Click **Load** and then **Import**
5. Repeat for `advanced_alerts.yaml`

For notification channels:

1. Navigate to **Alerting** → **Contact points**
2. Click **Add contact point**
3. Configure based on `notification_channels.yaml`

### 4. Configure Prometheus

Ensure your Prometheus is configured to scrape the necessary metrics:

```yaml
# prometheus.yml
scrape_configs:
  - job_name: 'orchestrator'
    static_configs:
      - targets: ['orchestrator:8080']
    metrics_path: /metrics
    scrape_interval: 15s

  - job_name: 'blackbox'
    metrics_path: /probe
    params:
      module: [orchestrator_health]
    static_configs:
      - targets:
        - http://orchestrator:8080/health
    relabel_configs:
      - source_labels: [__address__]
        target_label: __param_target
      - source_labels: [__param_target]
        target_label: instance
      - target_label: __address__
        replacement: blackbox-exporter:9115
```

## Alert Severity Levels

- **Critical**: Immediate action required, potential service disruption
- **Warning**: Attention needed, may escalate if not addressed
- **Info**: Informational, no immediate action required

## Alert Response Guidelines

### Critical Alerts Response

1. **Orchestrator Service Down**
   - Check container/pod status
   - Review recent deployments
   - Check system resources (CPU, Memory)
   - Review error logs

2. **No Block Progress**
   - Check all job queues
   - Verify external services (prover, DA, settlement)
   - Review database connections
   - Check for deadlocks

3. **High Failure Rates**
   - Identify failing job type
   - Check external dependencies
   - Review recent code changes
   - Consider rollback if necessary

### Warning Alerts Response

1. **Performance Degradation**
   - Monitor trend over time
   - Check system resources
   - Review database performance
   - Consider scaling if persistent

2. **Queue Depth Building**
   - Check worker health
   - Consider scaling workers
   - Review processing bottlenecks
   - Monitor for escalation

## Testing Alerts

### Simulate Alert Conditions

```bash
# Test service down alert
docker stop orchestrator

# Test high failure rate
# Run metrics example with forced failures

# Test queue depth alert
# Generate load without processing

# Test database slowness
# Add artificial delay to MongoDB queries
```

### Verify Notifications

1. Check Slack channels for test alerts
2. Verify PagerDuty incidents are created
3. Confirm email delivery
4. Test webhook endpoints

## Maintenance Windows

Configure mute timings to prevent alerts during scheduled maintenance:

```yaml
# Weekend maintenance: Saturday & Sunday 2-6 AM UTC
# Scheduled maintenance: Wednesday 3-4 AM UTC
```

## Custom Metrics for Alerts

If you need to add custom metrics for alerting:

```rust
// In orchestrator code
ORCHESTRATOR_METRICS.custom_metric.record(
    value,
    &[KeyValue::new("label", "value")]
);
```

Then create corresponding alert rules.

## Troubleshooting

### Alerts Not Firing

- Check Prometheus is scraping metrics
- Verify alert rule syntax
- Check datasource UID matches
- Review alert conditions and thresholds

### Notifications Not Received

- Verify webhook URLs are correct
- Check network connectivity
- Review notification channel configuration
- Check Grafana logs for delivery errors

### False Positives

- Adjust thresholds based on baseline
- Increase evaluation period (for parameter)
- Add additional conditions
- Use mute timings for known issues

## Best Practices

1. **Alert Fatigue Prevention**
   - Set appropriate thresholds
   - Use proper severity levels
   - Implement alert deduplication
   - Regular alert review and tuning

2. **Documentation**
   - Maintain runbooks for each alert
   - Document resolution steps
   - Keep contact information updated
   - Regular drill exercises

3. **Continuous Improvement**
   - Review alert effectiveness monthly
   - Adjust thresholds based on patterns
   - Remove obsolete alerts
   - Add new alerts for emerging issues

## Support

For issues or questions about alerts:

- Check orchestrator logs: `kubectl logs -n madara orchestrator`
- Review Grafana alert state: Alerting → Alert rules
- Contact: <orchestrator-team@madara.io>
