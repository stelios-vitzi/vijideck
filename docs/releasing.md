# Public release checklist

- Review the complete tracked tree and Git history for credentials, private data,
  internal URLs, screenshots, and accidental artifacts. Rotate exposed credentials;
  deleting a file does not remove it from history. Confirm redistribution rights
  for original artwork and all contributed material.
- Confirm that LICENSE (MIT) and the copyright line are current.
- Audit the exact locked dependency graph and native/bundled components for license
  compatibility, attribution, source-offer, and other redistribution requirements.
  Update THIRD_PARTY_NOTICES.md and package notice payloads with the full required
  texts. The included Devicon notice alone is not a complete dependency audit.
- Run all checks in CONTRIBUTING.md and the native CI packaging matrix. Inspect
  each package's license/notice payload and exercise install, launch, and uninstall.
  Test destructive operations only against disposable data and Docker resources.
- Verify README links and supported-platform claims. Enable GitHub private
  vulnerability reporting and repository security features where available.
- Update the root Cargo.toml version and Cargo.lock together. Keep xtask's version
  aligned if releasing it as part of the same version. Push a matching `v<version>`
  tag only after the reviewed changes are committed.
- The tag push runs the release workflow, which publishes the GitHub Release as
  latest once the version check, platform jobs, and RustSec audit succeed. Download
  a CI artifact from a branch build first to inspect packages before tagging, and
  afterwards check the packages, combined SHA256SUMS.txt, release notes, known
  issues, and unsigned distribution notice. Checksums prove integrity, not identity.
- Configure Windows signing and macOS Developer ID/notarization when available;
  choose a macOS bundle identifier under your control before signing. See
  [development and packaging](development.md) for current packaging limitations.

Making the repository public and publishing a release are separate actions from
preparing these files locally.
