# Key and secret runbook

Two secrets protect shipped functionality. An optional certificate can also
add Windows publisher identity. The procedures below keep those roles separate.

## What exists

| Secret | Where it lives | What it signs or unlocks | If it is lost | If it leaks |
| --- | --- | --- | --- | --- |
| `MINISIGN_SECRET_KEY` | Maintainer's offline backup; GitHub Actions secret | `latest.json` and every release artifact | The update channel stops for every installed copy | Anyone can publish a manifest installed copies will trust |
| `OCCLUVIEW_SIGN_PFX_BASE64` (+ `OCCLUVIEW_SIGN_PFX_PASSWORD`) or `OCCLUVIEW_SIGN_CERT_SHA1` | Optional certificate authority issuance; GitHub Actions secret | Authenticode on the MSI, viewer and shell DLL | New artifacts ship without Windows publisher identity but retain minisign verification | Revoke through the issuing CA; Authenticode timestamps limit the blast radius |
| `OCCLUVIEW_HPS_EMBEDDED_KEY` | Maintainer's offline backup; GitHub Actions secret | Encrypted dental containers in official builds | Official builds can no longer open encrypted containers | It is obfuscation, not a secret boundary — see `docs/ARCHITECTURE.md` |

The public half of the first one is committed as `occluview.pub` and compiled
into the updater as `UPDATE_PUBKEY`. The publish job re-verifies every signature
against that constant before uploading, so the signing key and the shipped key
cannot drift apart without the release failing.

## Rotating the update key

The trust anchor is compiled into binaries already installed on workstations, so
a rotation cannot be a single event. It is two ordinary releases:

1. Generate the next key pair offline: `minisign -G -p next.pub -s next.key`.
2. Add the new public key to `UPDATE_PUBKEYS` in
   `crates/occluview-update/src/lib.rs`, leaving `UPDATE_PUBKEY` — the signing
   key — unchanged. Release. Installed copies now accept both.
3. Wait until that release has spread. Anything still on an older build will
   stop updating at step 4, and will need the manual path below.
4. Replace the `MINISIGN_SECRET_KEY` secret with the new private key, set
   `UPDATE_PUBKEY` to the new public key, drop the old entry from
   `UPDATE_PUBKEYS`, and update `occluview.pub` and the key quoted in the
   README. Release.

Skipping step 2 is what turns a rotation into an outage.

## If the update channel is already dead

Symptom: installed copies report "update signature verification failed", or stop
offering updates while the release page has newer versions.

1. Do not re-sign the existing release with a different key. Installed copies do
   not re-fetch a manifest they have already rejected, and a second key they do
   not trust changes nothing.
2. Publish the new version normally so the release page is correct.
3. Tell users to download and install manually, and point them at the
   *Verify your download* section of the README so the manual path is still a
   verified one.
4. Rotate forward with the procedure above so the next release repairs the
   channel for everyone who installs manually once.

## Certificate expiry

Authenticode signatures are timestamped, so already-shipped artifacts stay
valid after the certificate expires. Renew before expiry to keep Windows
publisher identity on new artifacts. Certificate availability does not replace
the mandatory minisign verification used by the updater.

## What is deliberately not here

No HSM, no signing ceremony, no key escrow with a third party. For a project
this size the cost of those exceeds the risk they remove, and the procedures
above are the part that actually gets used.
