# The Hermes <version>: release notes

DevOps fills this in for each release decision. The owner reads it before ruling. Keep every section; write "none" where nothing applies.

## What's in it
- <item id>: <title> (<platform>)

## Builds
| Platform | Version | sha256 | Signed by |
|---|---|---|---|
| desktop (macOS) | <version> | <sha256> | Developer ID, team <TEAMID> |
| desktop (Windows) | <version> | <sha256> | unsigned / <signer> |
| ios | <version> | <sha256> | development profile |

`hermesd --version` on the built daemon says: `identity: <line>` (H-114).

## How to test
<steps per platform, as in the package's how-to-test>

## Rollout
- Testers install with `hermesd release install <release>` from their own session.
  - It checks the gate, the sha256 and the signature before anything changes.
  - It hands the service install to the system, so the tester's session restarts with the service.
  - From the next session, `--status` shows how it went; then `deploy_confirm`.
- Computers: <required machines>.

## Security notes
- **Installer deny rules are advisory only.** Non-DevOps bots' settings deny direct installer and service commands (`hermesd service install`, `installer -pkg`, `msiexec`, …). They match command text, so they are guidance, not a boundary. The gate the daemon checks (`install_release`, the owner's approval, the frozen hash, the signature) is what protects an install.
- Signature checks skipped in this release: <none / which builds, and why>.

## Rolling back
<what DevOps does, and which data a downgrade needs, e.g. re-pairing phones after a phase-2 downgrade>
