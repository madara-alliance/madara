# Kubernetes Manifests for Orchestrator

This directory contains Kubernetes manifests for deploying the Madara Orchestrator using Kustomize.

## Directory Structure

```
infra/k8s/
├── base/                           # Base manifests
│   ├── orchestrator.yaml          # Deployment manifest
│   ├── configmap.yaml             # ConfigMap with environment variables
│   ├── secrets.yaml               # Secret for sensitive data
│   ├── svc.yaml                   # Service manifest
│   └── kustomization.yaml         # Base kustomization
├── overlays/                      # Environment-specific overlays
│   ├── dev/                       # Development environment
│   │   ├── kustomization.yaml
│   │   └── deployment-patch.yaml
│   ├── staging/                   # Staging environment
│   │   ├── kustomization.yaml
│   │   └── deployment-patch.yaml
│   └── production/                # Production environment
│       ├── kustomization.yaml
│       └── deployment-patch.yaml
├── deployment.yaml                # Reference deployment (do not modify)
├── cm.yaml                        # Reference configmap (do not modify)
└── README.md                      # This file
```

## Usage

### Deploy to Development

```bash
kubectl apply -k infra/k8s/overlays/dev
```

### Deploy to Staging

```bash
kubectl apply -k infra/k8s/overlays/staging
```

### Deploy to Production

```bash
kubectl apply -k infra/k8s/overlays/production
```

### Preview Generated Manifests

```bash
kubectl kustomize infra/k8s/overlays/dev
```

## Environment Configuration

Each overlay configures the following:

### Development (dev)

- **Namespace**: `dev`
- **Replicas**: 1
- **Image Tag**: `latest`
- **Resources**: 2 CPU, 4Gi memory
- **Name Prefix**: `dev-`
- **Environment**: Debug logging enabled

### Staging (staging)

- **Namespace**: `staging`
- **Replicas**: 2
- **Image Tag**: `staging`
- **Resources**: 4 CPU, 8Gi memory
- **Name Prefix**: `staging-`
- **Environment**: Info logging

### Production (production)

- **Namespace**: `production`
- **Replicas**: 3
- **Image Tag**: `v1.0.0`
- **Resources**: 6 CPU, 12Gi memory
- **Name Prefix**: `prod-`
- **Environment**: Info logging with audit artifacts enabled

## Customization

### Changing Image Tag

Edit the overlay's `kustomization.yaml`:

```yaml
images:
  - name: ghcr.io/madara-alliance/orchestrator
    newTag: your-new-tag
```

### Changing Replica Count

Edit the overlay's `kustomization.yaml`:

```yaml
replicas:
  - name: orchestrator
    count: 5
```

### Changing Namespace

Edit the overlay's `kustomization.yaml`:

```yaml
namespace: your-namespace
```

### Adding Environment Variables

For non-sensitive configuration, edit the overlay's `kustomization.yaml`:

```yaml
configMapGenerator:
  - name: madara-orchestrator-config
    behavior: merge
    literals:
      - YOUR_ENV_VAR=value
```

### Managing Secrets

Sensitive data like API keys, passwords, and credentials are stored in Kubernetes Secrets. Edit the overlay's `kustomization.yaml` to update secret values:

```yaml
secretGenerator:
  - name: madara-orchestrator-secrets
    behavior: merge
    literals:
      - AWS_ACCESS_KEY_ID=your-access-key
      - AWS_SECRET_ACCESS_KEY=your-secret-key
      - MADARA_ORCHESTRATOR_ATLANTIC_API_KEY=your-api-key
      - MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL=your-mongodb-url
```

**Security Best Practice**: Never commit actual secret values to version control. Use one of these methods:

1. **External Secret Management**: Use tools like sealed-secrets, external-secrets, or Vault
2. **Local Override**: Create a `secrets-local.yaml` patch file (add to `.gitignore`) and reference it in your kustomization
3. **CI/CD Injection**: Inject secret values during deployment via CI/CD pipelines

### Modifying Resources

Edit the overlay's `deployment-patch.yaml`:

```yaml
spec:
  template:
    spec:
      containers:
        - name: madara-orchestrator
          resources:
            limits:
              cpu: "8"
              memory: 16Gi
            requests:
              cpu: "8"
              memory: 16Gi
```

## Important Notes

1. **Secrets Management**: Sensitive data is separated into Kubernetes Secrets (`secrets.yaml`). The following values are stored as secrets:
   - `AWS_ACCESS_KEY_ID`
   - `AWS_SECRET_ACCESS_KEY`
   - `MADARA_ORCHESTRATOR_ATLANTIC_API_KEY`
   - `MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL`

2. **ConfigMap vs Secrets**:
   - Use **ConfigMap** for non-sensitive configuration (URLs, feature flags, limits, etc.)
   - Use **Secrets** for sensitive data (credentials, API keys, private keys, connection strings)

3. **Base Manifests**: Always modify the base manifests (`base/`) for changes that should apply to all environments.

4. **Overlay Patches**: Use overlay-specific patches for environment-specific configurations.

## Required Configuration Updates

Before deploying, update these values in your overlay:

### Required Secrets (via `secretGenerator`):

- `AWS_ACCESS_KEY_ID` - AWS access key for S3/SQS/SNS
- `AWS_SECRET_ACCESS_KEY` - AWS secret key
- `MADARA_ORCHESTRATOR_ATLANTIC_API_KEY` - Atlantic service API key
- `MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL` - MongoDB connection string

### Required ConfigMap Values (via `configMapGenerator`):

- `MADARA_ORCHESTRATOR_ATLANTIC_RPC_NODE_URL` - RPC node URL
- `MADARA_ORCHESTRATOR_ATLANTIC_VERIFIER_CONTRACT_ADDRESS` - Verifier contract address
- `MADARA_ORCHESTRATOR_AWS_PREFIX` - AWS resource prefix
- `MADARA_ORCHESTRATOR_AWS_S3_BUCKET_IDENTIFIER` - S3 bucket name
- `MADARA_ORCHESTRATOR_DATABASE_NAME` - Database name
- `MADARA_ORCHESTRATOR_ETHEREUM_DA_RPC_URL` - Ethereum DA RPC URL
- `MADARA_ORCHESTRATOR_ETHEREUM_PRIVATE_KEY` - Ethereum private key
- `MADARA_ORCHESTRATOR_ETHEREUM_SETTLEMENT_RPC_URL` - Ethereum settlement RPC
- `MADARA_ORCHESTRATOR_L1_CORE_CONTRACT_ADDRESS` - L1 core contract address
- `MADARA_ORCHESTRATOR_MADARA_RPC_URL` - Madara RPC URL
- And other environment-specific values

## Verification

After deployment, verify the resources:

```bash
# Check deployment
kubectl get deployments -n <namespace>

# Check pods
kubectl get pods -n <namespace>

# Check service
kubectl get svc -n <namespace>

# Check configmap
kubectl get configmap -n <namespace>

# Check secrets
kubectl get secrets -n <namespace>

# View secret values (decode base64)
kubectl get secret <secret-name> -n <namespace> -o jsonpath='{.data}' | jq -r 'to_entries[] | "\(.key): \(.value | @base64d)"'
```

## Example: Deploying to Development

1. Update secrets in `overlays/dev/kustomization.yaml`:

   ```yaml
   secretGenerator:
     - name: madara-orchestrator-secrets
       behavior: merge
       literals:
         - AWS_ACCESS_KEY_ID=your-dev-access-key
         - AWS_SECRET_ACCESS_KEY=your-dev-secret-key
         - MADARA_ORCHESTRATOR_ATLANTIC_API_KEY=your-dev-api-key
         - MADARA_ORCHESTRATOR_MONGODB_CONNECTION_URL=mongodb://dev-url
   ```

2. Update environment-specific ConfigMap values if needed

3. Preview the generated manifests:

   ```bash
   kubectl kustomize infra/k8s/overlays/dev
   ```

4. Deploy:

   ```bash
   kubectl apply -k infra/k8s/overlays/dev
   ```

5. Verify:
   ```bash
   kubectl get all -n dev
   ```
