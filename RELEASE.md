# Release process

## 1. Tag it

Once ready, tag the `main` branch at the given commit with the release tag:

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

## 2. Publish the GitHub Release

Create and publish a GitHub Release for the pushed tag. Publishing the release
triggers `.github/workflows/release-publish.yml`, which builds and publishes
the release container images.

## 3. Verify the Release Workflow

Check the "Workflow - Release (publish)" run and confirm the release image jobs
completed successfully.
