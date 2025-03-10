# Release process

1. Once ready, tag the "main" branch at the given commit with the right tag:

```bash
git tag v0.8.7
```

2. Trigger the manual workflow "Manual Release with Binaries" (manual_release.yml)

- `tag`: v0.8.7
- base: main

3. Review the release draft and publish
4. Review the CHANGELOG.md Pull Request and merge
