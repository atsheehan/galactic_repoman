# SteamPipe publishing

The **GitHub Actions workflow (`.github/workflows/release.yml`) is the real and
primary way `galactic_repoman` is published.** The local steamcmd flow below is a
testing aid for validating staging and Steam connectivity by hand.

Every run auto-promotes the build to a dedicated, non-default beta branch
(`internal`, the `branch` input default) so the developer's opted-in Steam client
installs the freshest build automatically. `default`/`public` — what real buyers
get once the game is released — is deliberately never targeted.

No third-party action is in the credential path: CI downloads steamcmd directly
from Valve's official CDN and runs it from our own scripts, so the builder
credential is seen only by Valve's steamcmd binary and our own YAML/shell.

## CI/CD (primary)

### Required secrets

| Secret                | Value                                                            |
| --------------------- | --------------------------------------------------------------- |
| `STEAM_USERNAME`      | Builder account login                                           |
| `STEAM_PASSWORD`      | Builder account password                                        |
| `STEAM_SHARED_SECRET` | Base64 authenticator seed (`shared_secret`) for the builder acct |

### Required variables

| Variable         | Value                |
| ---------------- | -------------------- |
| `STEAM_APP_ID`   | The Steam AppID      |
| `STEAM_DEPOT_ID` | The depot's DepotID  |

### Obtaining `STEAM_SHARED_SECRET`

`shared_secret` is the base64 authenticator seed used to generate Steam Guard
codes. Steam hands it out only when an authenticator is first registered to the
account; the mobile app stores it on-device but doesn't expose it. Steam Desktop
Authenticator is no longer maintained, so use
[steamguard-cli](https://github.com/dyc3/steamguard-cli) (actively maintained,
Linux/Windows/macOS).

Run against the **builder account** (not your personal login):

```
steamguard setup   # registers a new authenticator; writes a .maFile
python3 -c "import json; print(json.load(open('<steamid>.maFile'))['shared_secret'])"
```

Copy that base64 value into the `STEAM_SHARED_SECRET` secret. Verify it before
trusting CI — `python3 scripts/steam_guard_totp.py "<shared_secret>"` should match
the code the authenticator currently shows.

> **Caveat:** an account holds only one authenticator at a time, and
> replacing an existing one triggers Steam's 15-day trade/market hold. That's
> why this must be a **dedicated builder account** — the hold is harmless there.
> Never run `setup` against your personal account.

Alternatives that avoid re-registration: read `shared_secret` from an existing
SDA `.maFile` if you still have one, or extract
`/data/data/com.valvesoftware.android.steam.community/files/Steamguard-<steamid64>`
from a **rooted** Android device (JSON with the same field).

Guard the secret like the password — it is masked in logs, but treat it as a
service credential.

### Running

Actions → **Release (SteamPipe)** → **Run workflow**. Leave `branch` as `internal`
(never set `default`/`public`), set a `description`, and dispatch. The workflow
builds the release binary, stages the depot content, renders our VDFs, generates a
Steam Guard code in our own step, and uploads via steamcmd.

## One-time Steam setup

1. On the Steamworks partner site, create the `internal` beta branch for the app
   (optionally password-protect it).
2. On the local Steam client, opt the game into `internal` via **Properties →
   Betas**, so CI promotions install automatically.

## Local testing (optional)

Validate staging and Steam connectivity by hand; reuses the same shared scripts
and VDF templates as CI.

1. `./scripts/bootstrap_steamcmd.sh`
2. `cp ci/steampipe/depot.env.example ci/steampipe/depot.env` and fill in the real
   AppID/DepotID (non-secret config). direnv users can instead add a gitignored
   `.envrc` with `dotenv ci/steampipe/depot.env`.
3. `./scripts/package.sh`
4. Upload (interactive Steam Guard prompt — just type the authenticator code; no
   shared secret needed locally):

   ```
   tools/steamcmd/steamcmd.sh +login <builder_account> \
     +run_app_build "$PWD/ci/steampipe/build/app_build_<APPID>.vdf" +quit
   ```

5. Promote on the partner site, or via the `SETLIVE` value in `depot.env`.

## Plan B: cached `config.vdf` (fallback auth)

If inline TOTP hits new-device 2FA friction, fall back to a cached login token:
log in once locally with `tools/steamcmd/steamcmd.sh +login <builder_account>`,
then base64 the resulting `config/config.vdf` from steamcmd's config dir into a
secret and restore it into the runner's steamcmd config dir before
`+login`. Simpler to bootstrap, but the cached token expires and needs periodic
manual refresh — that's why inline TOTP is the chosen primary method.

## steamcmd download integrity

Valve publishes no SHA256/GPG checksum for `steamcmd_linux.tar.gz`, and steamcmd
is a self-updating bootstrapper (a pinned tarball hash wouldn't cover the client
it downloads on first run). Trust is HTTPS-to-Valve's-CDN.
