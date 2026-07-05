# Step 2 — SteamPipe Publishing (CI/CD-first, no third-party actions in the credential path)

## Context

`galactic_repoman` has completed Step 1: a self-contained release binary (`galactic_repoman`)
that opens a window and renders a Vulkan triangle. Shaders are compiled into the binary at build
time, so **the depot content root is currently just that one ELF binary** — no assets, no `.spv`,
no Steam libs yet (those layer in during Steps 3–4).

Step 2 proves the content-delivery + SteamOS runtime path early. The **GitHub Actions CI/CD
workflow is the real (and only) way the game gets published**; the local steamcmd flow is kept as a
**testing aid** to validate staging and Steam connectivity by hand.

**Distribution intent (pre-release):** the app is registered but unreleased — no store page, no
public license — so builds are visible only to accounts with access to *this app* (the developer's
comp license). The goal is for **every CI run to auto-promote the build to a dedicated, non-default
beta branch (`internal`)** that the developer's local Steam client opts into, so the freshest build
installs automatically. `default`/`public` is deliberately never targeted: it's what real buyers get
once the game is released, so an automated triangle pipeline must stay off it. `setlive` is a
**branch name**, not a boolean — auto-promotion means setting it to `internal`, never `true`.

**Security decision (user-driven):** publishing handles a Steam builder-account credential, so we
**do not route it through any third-party GitHub Action.** There is no Valve-official action or
Docker image — `game-ci/steam-deploy`, `steamcmd/steamcmd`, and `cm2network/steamcmd` are all
community projects. The only thing Valve officially publishes is the steamcmd binary itself (from
its CDN). So CI **downloads steamcmd directly from Valve's official URL and runs it from our own
scripts** — the credential is seen only by (a) Valve's steamcmd binary and (b) our own YAML/shell.

Other locked decisions:

- **Primary deliverable = the CI/CD release workflow.** Local flow is secondary/testing-only.
- **AppID/DepotID**: template + substitution — `__APPID__`/`__DEPOTID__` tokens in committed VDF
  templates, real values from a gitignored `ci/steampipe/depot.env` (local) or repo **variables**
  (CI). The hand-written VDFs are the single source of truth, used by **both** local and CI. These
  are **non-secret** config; `depot.env` stays canonical, and `direnv` users can load it for free
  (`package.sh` reads the environment when `depot.env` is absent — see direnv note below).
- **CI auth = inline first-party TOTP** (user choice): a small `scripts/steam_guard_totp.py` we own
  generates the Steam Guard code from a `STEAM_SHARED_SECRET` GitHub secret each run — no
  third-party action and no expiring cached token.

## Provenance findings (why the approach is what it is)

- **No Valve-official steamcmd Action or Docker image exists.** Valve publishes only the steamcmd
  binary at `https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz` and the SDK
  ContentBuilder (partner-login gated).
- `game-ci/steam-deploy` = community (GameCI org); internally wraps `CyberAndrii/setup-steamcmd` +
  `CyberAndrii/steam-totp` — multiple third parties in the credential path. Rejected on that basis.
- `steamcmd/steamcmd` / `cm2network/steamcmd` = community Docker images wrapping Valve's binary —
  not Valve, so they don't add trust over downloading the binary ourselves.
- **Chosen**: self-hosted steamcmd from Valve's official CDN, invoked by our own scripts.

## Deliverables (file tree)

```
.github/workflows/
  release.yml                 # PRIMARY: manual-dispatch publish; self-hosted steamcmd, our VDFs
ci/steampipe/
  app_build.vdf.template      # SHARED source of truth: tokens __APPID__/__DEPOTID__/__SETLIVE__
  depot_build.vdf.template    # SHARED: token __DEPOTID__
  depot.env.example           # committed sample for the local testing flow
  README.md                   # CI secrets/variables setup + local testing recipe
scripts/
  package.sh                  # SHARED: cargo build --release -> stage content -> render VDFs
  bootstrap_steamcmd.sh       # SHARED: download steamcmd from Valve CDN into ./tools/steamcmd/
  steam_guard_totp.py         # CI auth: first-party Steam Guard TOTP from STEAM_SHARED_SECRET
.gitignore                    # additions (build/, depot.env, tools/, steam_appid.txt)
```

## Shared building blocks (used by both CI and local)

### `ci/steampipe/app_build.vdf.template`

SteamPipe app-build script. `setlive` carries the **branch name to auto-promote to** (rendered from
`SETLIVE`, default `internal`); it is never `default`/`public`. `contentroot`/`buildoutput` are
relative to the rendered VDF's location (`ci/steampipe/build/`).

```
"appbuild"
{
	"appid"		"__APPID__"
	"desc"		"galactic_repoman scaffold (triangle-only)"
	"buildoutput"	"output/"
	"contentroot"	"content/"
	"setlive"	"__SETLIVE__"
	"depots"  { "__DEPOTID__"  "depot_build___DEPOTID__.vdf" }
}
```

### `ci/steampipe/depot_build.vdf.template`

```
"DepotBuild"
{
	"DepotID"	"__DEPOTID__"
	"FileMapping"  { "LocalPath" "*"  "DepotPath" "."  "recursive" "1" }
	"FileExclusion"	"*.pdb"
}
```

### `scripts/package.sh`

`#!/usr/bin/env bash`, `set -euo pipefail`. Parameterized by `APPID`/`DEPOTID`/`SETLIVE`, taken
from `ci/steampipe/depot.env` **if present, else from the already-exported environment** (so CI
supplies them from repo variables). Steps: `cargo build --release`; reset
`ci/steampipe/build/{content,output}/`; copy `target/release/galactic_repoman` into `content/`
(preserve the `+x` bit); render both `.vdf.template`s via `sed` token substitution into
`build/app_build_<APPID>.vdf` and `build/depot_build_<DEPOTID>.vdf`. This single script does the
staging for **both** paths.

### `scripts/bootstrap_steamcmd.sh`

`set -euo pipefail`. `curl -sqL https://steamcdn-a.akamaihd.net/client/installer/steamcmd_linux.tar.gz | tar -xzf -`
into `./tools/steamcmd/` (gitignored); idempotent. Used locally and in CI so the binary always
comes from Valve's official URL, not a third party.

### `scripts/steam_guard_totp.py` (CI auth)

First-party Steam Guard TOTP generator — no third-party dependency, stdlib only. Note Steam's TOTP
is **not** RFC-6238 numeric: it uses a 5-char code over the alphabet `23456789BCDFGHJKMNPQRTVWXY`.

```python
#!/usr/bin/env python3
import base64, hmac, hashlib, struct, sys, time
def steam_guard_code(shared_secret, for_time=None):
    key = base64.b64decode(shared_secret)
    counter = int((for_time if for_time is not None else time.time()) // 30)
    digest = hmac.new(key, struct.pack(">Q", counter), hashlib.sha1).digest()
    offset = digest[19] & 0x0F
    code_int = struct.unpack(">I", digest[offset:offset + 4])[0] & 0x7FFFFFFF
    alphabet = "23456789BCDFGHJKMNPQRTVWXY"
    out = ""
    for _ in range(5):
        out += alphabet[code_int % len(alphabet)]
        code_int //= len(alphabet)
    return out
if __name__ == "__main__":
    print(steam_guard_code(sys.argv[1] if len(sys.argv) > 1 else __import__("os").environ["STEAM_SHARED_SECRET"]))
```

Runs on the runner's stock `python3` (Steam TOTP needs an accurate clock — GitHub runners are
NTP-synced, so no offset handling needed).

### `ci/steampipe/depot.env.example` (committed)

`APPID=`, `DEPOTID=`, `SETLIVE=` with a header comment to copy to gitignored `ci/steampipe/depot.env`.
**direnv (optional):** since `package.sh` reads these from the environment when `depot.env` is
absent, a gitignored `.envrc` containing `dotenv ci/steampipe/depot.env` lets direnv users load the
config on `cd`. No code change, no tool requirement — `depot.env` remains the canonical mechanism.

## PRIMARY: `.github/workflows/release.yml`

Mirrors `ci.yml` hardening: third-party actions pinned to full commit SHAs,
`permissions: contents: read`, `CARGO_TERM_COLOR: always`. **No third-party action receives the
Steam credentials** — checkout/rust actions never get them; the secret only touches our own `run:`
steps and Valve's steamcmd.

- **Trigger**: `workflow_dispatch` only, inputs `branch` (Steam beta branch to auto-promote to;
  **default `internal`**, never `default`/`public`) and `description` (build description).
- **Concurrency**: `group: steam-release`, `cancel-in-progress: false` (no racing the depot).
- **Single job** (`ubuntu-latest`):
  1. `actions/checkout` (SHA-pinned).
  2. `dtolnay/rust-toolchain` stable + `Swatinem/rust-cache` (SHA-pinned).
  3. `sudo apt-get update && sudo apt-get install -y lib32gcc-s1` (steamcmd bootstrapper needs the
     32-bit runtime on a clean runner).
  4. `./scripts/bootstrap_steamcmd.sh` (downloads steamcmd from Valve's CDN).
  5. `./scripts/package.sh` with `APPID`/`DEPOTID`/`SETLIVE` exported from
     `${{ vars.STEAM_APP_ID }}` / `${{ vars.STEAM_DEPOT_ID }}` / `${{ inputs.branch }}` — builds,
     stages, and renders **our** VDFs.
  6. Generate the Steam Guard code in our own step:
     `CODE=$(python3 scripts/steam_guard_totp.py)` with `STEAM_SHARED_SECRET` in the step `env`
     (mask it via `::add-mask::`).
  7. Run the upload from our own step — provide the code with `+set_steam_guard_code` **before**
     `+login` (steamcmd does not reliably accept a mobile code as a positional `+login` arg):
     `tools/steamcmd/steamcmd.sh +set_steam_guard_code "$CODE" +login "$STEAM_USERNAME" "$STEAM_PASSWORD" +run_app_build "$PWD/ci/steampipe/build/app_build_<APPID>.vdf" +quit`
     (username/password/secret are passed only to our scripts and Valve's binary, in steps we own).
- **Required secrets**: `STEAM_USERNAME`, `STEAM_PASSWORD`, `STEAM_SHARED_SECRET` (the base64
  authenticator seed for the builder account).
- **Required variables**: `STEAM_APP_ID`, `STEAM_DEPOT_ID`.

> Exact pinned SHAs for checkout/rust-toolchain/rust-cache are resolved at implementation time
> (look up current release SHAs; do not fabricate them), per the `ci.yml` convention. None of these
> actions are in the credential path.

## SECONDARY (testing only): local steamcmd flow

Validate staging and Steam connectivity by hand; reuses the shared blocks above.

1. `./scripts/bootstrap_steamcmd.sh`.
2. `cp ci/steampipe/depot.env.example ci/steampipe/depot.env` and fill in real AppID/DepotID.
3. `./scripts/package.sh`.
4. Upload (interactive Steam Guard, once per machine):
   `tools/steamcmd/steamcmd.sh +login <builder_account> +run_app_build "$PWD/ci/steampipe/build/app_build_<APPID>.vdf" +quit`
5. Promote on the partner site or via `SETLIVE`. (Local login uses the interactive Steam Guard
   prompt — just type the authenticator code; no shared secret needed locally.)

### `ci/steampipe/README.md`

Three sections: **CI/CD (primary)** — the secrets/variables list, how to obtain the builder
account's `shared_secret` (from the Steam Desktop Authenticator / mobile authenticator export) for
`STEAM_SHARED_SECRET`, and how to run the workflow; **One-time Steam setup** — create the `internal`
beta branch on the Steamworks partner site (optionally password-protect it), then on the local Steam
client opt the game into it via Properties → Betas, so CI promotions install automatically; **Local
testing (optional)** — the five steps above.

### `.gitignore` additions

```
# SteamPipe
/tools/steamcmd/
/ci/steampipe/build/
/ci/steampipe/depot.env
steam_appid.txt
```

## Auth method note (chosen: inline TOTP)

- **Chosen: inline first-party TOTP** (`scripts/steam_guard_totp.py` from a `STEAM_SHARED_SECRET`
  secret). No third-party action, no expiring cached token; the code is regenerated every run.
- **Fallback if needed:** base64 `config.vdf` secret (one local login, base64'd) restored into
  steamcmd's config dir. Simpler to bootstrap but the cached token expires and needs periodic manual
  refresh. Kept documented in the README as plan B.

## steamcmd download integrity note

Valve publishes **no** SHA256/GPG checksum for `steamcmd_linux.tar.gz` (confirmed open request on
`ValveSoftware/steam-for-linux`); trust is HTTPS-to-Valve's-CDN. steamcmd is also a self-updating
bootstrapper, so pinning the tarball hash wouldn't cover the client it downloads on first run.
`bootstrap_steamcmd.sh` *may* assert a self-computed hash as a tamper tripwire, but its value is
limited and it'll need bumping whenever Valve rev's the launcher — treat as optional, low-priority.

## What is intentionally NOT in this step

- No `assets/`, `libsteam_api.so`, `steam_appid.txt`, or IGA manifest in the content root — the
  binary has no Steam integration yet. Steps 3–4 add those to the same `package.sh` staging + VDFs.
- **Sniper runtime parity** (building inside `registry.gitlab.steamos.cloud/steamrt/sniper/sdk`)
  stays the flagged hardening follow-up; the CI build job can later gain a `container:` for it.

## Verification

**CI/CD (primary):**

1. Add secrets (`STEAM_USERNAME`, `STEAM_PASSWORD`, `STEAM_SHARED_SECRET`) and variables
   (`STEAM_APP_ID`, `STEAM_DEPOT_ID`).
2. Run `release.yml` via **workflow_dispatch** (branch = `internal` default, a description).
3. Confirm: steamcmd downloads from Valve's CDN, the generated Guard code authenticates with no
   interactive prompt, and the build appears on the Steamworks partner **Builds** page **and is set
   live on `internal`** (not `default`).
4. With the local Steam client opted into `internal`, confirm it auto-updates, launches, and renders
   the triangle in shipped form. Re-run the workflow and confirm the client picks up the new build
   automatically. (Satisfies the master plan's Step 2 exit criteria via CI.)

**Local (testing aid):**

- Run the five-step local flow; confirm `ci/steampipe/build/content/galactic_repoman` exists with
  `+x` and the rendered VDFs contain real IDs (no `__TOKENS__` left); confirm the manual upload
  reaches the partner Builds page.

## Risks / call-outs

- **New-device 2FA friction**: a valid TOTP code normally satisfies Steam Guard, but Steam may add
  one-time "approve this new device" friction the first time the builder account logs in from a
  fresh runner/IP. If hit, do one interactive local/`workflow_dispatch` login to establish trust, or
  fall back to the documented `config.vdf` method. Flagged to validate during first CI run.
- **Clock skew**: Steam TOTP is time-based; relies on the runner clock (GitHub runners are
  NTP-synced, so fine) — only a concern for self-hosted runners.
- **Builder account hygiene**: dedicated account with Steamworks access to the app's depots; treat
  it as a service credential, not a personal login. `STEAM_SHARED_SECRET` is the authenticator seed
  — guard it like the password and mask it in logs.
- **steamcmd download integrity**: no Valve checksum exists (see note above) — trust is HTTPS to
  Valve's CDN.
- **steamcmd 32-bit dependency** on clean runners (`lib32gcc-s1`) — included as a workflow step.
- **Never auto-promote to the default branch**: the `branch` input / `SETLIVE` default is `internal`
  (a non-default beta branch), and `default`/`public` must never be the target — that's the branch
  real buyers get once the game is released. Enforced by the input default; treat passing `default`
  as a mistake.
- **Linux executable bit**: Steam tracks it on Linux depots; verify the staged binary keeps `+x`.
