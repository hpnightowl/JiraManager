# Releasing and signing

Goal: anyone who installs ProjectJira can check that the file really came from this repository's
pipeline and was not changed afterwards.

## What "signed" means here
| Platform | How | What it proves |
|---|---|---|
| Windows `.exe` / `.msi` | **Authenticode** signature made in the pipeline | Publisher identity; Windows warns if the file was altered; far fewer SmartScreen warnings. |
| Every file (Linux `.deb`, Windows) | **GPG detached signatures** + signed `SHA256SUMS` | The file is byte-for-byte what the pipeline built, signed by a key only the pipeline holds. |

Signing cannot stop someone who already has write access to the repository from releasing bad code.
That is what the protections below are for (reviewer approval, protected branch and tags).

## One-time setup (about 30 minutes)

### 1. Protect the repository (GitHub > Settings)
- **Rules > Rulesets > New branch ruleset** for `main`: block force-push and deletion, require the
  `CI` checks to pass. (Private repositories on the free plan may not offer rulesets; the same options
  exist under *Branches > Branch protection rules*. If neither is offered, you are on a plan without
  them: be extra careful with who has write access.)
- **New tag ruleset** for `v*`: block deletion and updates, restrict creation to yourself.
- **Environments > New environment `release`**: add yourself as **required reviewer**, and limit
  deployment to tags matching `v*`. The signing secrets go here, not in "Repository secrets", so a
  pull request or a random branch can never read them.
- **Plan limits:** for a *private* repository, rulesets/branch protection and environment reviewers
  need GitHub Pro or Team (they are free on public repositories). On the Free plan the secrets still
  work, but nothing forces an approval step: keep the release secrets out of the repository until you
  can turn those protections on, or make the repository public once it is reviewed.
- **Actions > General > Workflow permissions**: *Read repository contents* (the workflows ask for more
  only where needed).

### 2. A GPG signing key (for all files)
On your own computer:
```bash
gpg --quick-generate-key "ProjectJira Releases <iamhp2k@gmail.com>" ed25519 sign 2y
gpg --list-secret-keys --keyid-format long          # note the key id
gpg --armor --export <KEYID> > release-public-key.asc            # PUBLIC part: commit this file
gpg --armor --export-secret-keys <KEYID>                          # PRIVATE part: paste into the secret below
```
Environment `release` > **Secrets**: `GPG_PRIVATE_KEY` (the private block) and `GPG_PASSPHRASE`.
Then delete any copy of the private key from your Downloads/clipboard history and keep an offline
backup (a USB stick). Commit `release-public-key.asc` and write its fingerprint in the README so
people can compare it.

### 3. Windows code signing: pick one
**Be aware of the market:** since June 2023 certificate authorities issue code-signing keys only on
hardware, so you can no longer download a `.pfx` from most of them. That leaves:

| Option | Cost | Notes |
|---|---|---|
| **Azure Artifact Signing** (formerly Trusted Signing) | about US$10 a month | No key file at all; the pipeline signs through the cloud. Eligibility is limited by country and account type (as of 2026: organisations in the US, Canada, EU, UK; individuals in the US and Canada, so check Microsoft's page for your country first). Set secrets `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_CLIENT_SECRET` and variables `ARTIFACT_SIGNING_ENDPOINT`, `ARTIFACT_SIGNING_ACCOUNT`, `ARTIFACT_SIGNING_PROFILE`. The workflow already handles it ("option B"). |
| **A cloud-HSM certificate** from a CA (SSL.com eSigner, DigiCert KeyLocker, Sectigo...) | roughly US$200-500 a year | Works for individuals in more countries, after identity checks. Each CA has its own signing tool; the workflow needs a small `signCommand` step for it (ask and it will be added once you choose one). |
| **An older `.pfx` certificate** you already own | - | The workflow handles it ("option A"): secrets `WINDOWS_CERTIFICATE` (base64 of the .pfx) and `WINDOWS_CERTIFICATE_PASSWORD`. |
| **No certificate** | free | The release is still built, GPG-signed and checksummed, but Windows shows a SmartScreen warning ("More info > Run anyway") and PCs with *Smart App Control* on refuse it. The release notes say **unsigned** so you don't mistake it for an official one. |

An OV certificate removes most warnings only as it gains reputation; an EV certificate removes them
at once. Neither is a guarantee.

### 4. First check
Push to `main`, watch **CI** go green, then create a **pre-release tag**:
```bash
git tag v0.1.0 && git push origin v0.1.0
```
The Release workflow runs, waits for your approval at the `release` environment, builds, signs and
creates a **draft** release. Open it, read the table at the top (signed / unsigned), download a file
and verify it yourself (below), then press **Publish**.

## Making a release
1. Change the version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` to the
   same number (the pipeline refuses a mismatch). Commit to `main` through CI.
2. `git tag vX.Y.Z && git push origin vX.Y.Z`
3. Approve the `release` environment when GitHub asks, wait for the draft, verify, publish.

## Verifying a download
```bash
gpg --import release-public-key.asc
gpg --verify SHA256SUMS.asc SHA256SUMS
sha256sum -c SHA256SUMS --ignore-missing
```
On Windows also: right-click the installer > Properties > Digital Signatures > Details.

## Extra assurance you can add later
- **Build provenance** (`actions/attest-build-provenance`) ties each file to the exact workflow run.
  GitHub offers it for private repositories only on the Enterprise Cloud plan; if the repository ever
  becomes public it is free. Until then the GPG signature does this job.
- **Reproducible builds / SBOM**: not done.

## If a key leaks
Delete the secret, revoke the GPG key (`gpg --gen-revoke`), generate a new key, commit the new public
key, and publish a notice. For a Windows certificate, ask the CA to revoke it.
