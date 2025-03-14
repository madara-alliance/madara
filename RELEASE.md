# Release process

## 1. Tag it

Once ready, tag the "main" branch at the given commit with the right tag:

```bash
git tag v0.8.7
```

## 2. Trigger the action

Trigger the manual workflow "Manual Release with Binaries" (manual_release.yml)

- `tag`: v0.8.7
- base: main

## 3. Publish the release

Review the release draft (created by the action) and publish

## 4. Merge the change logs

Review the CHANGELOG.md Pull Request (created by the action) and merge
