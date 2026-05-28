# Flipping `strivo-plugins` to a private repository

The plugins that ship with Strivo Pro live in a Git submodule at
`strivo-plugins/`. To gate them on a paid licence, the repo itself is
flipped to **private** under `Chorosyne/strivo-plugins`. Public builds
of StriVo continue to compile by checking out a no-op stub submodule
when the deploy key isn't present.

This document is the runbook. Execute it once, in order.

## 1. Flip the repo on GitHub

1. Open `https://github.com/Chorosyne/strivo-plugins/settings`.
2. Scroll to **Danger Zone** → **Change repository visibility** →
   **Private**.
3. Confirm by typing the repo name.

## 2. Create a read-only deploy key

```bash
# Generate a fresh, dedicated keypair for CI. Do NOT reuse a personal key.
ssh-keygen -t ed25519 -f strivo-plugins-deploy -N "" \
  -C "strivo-plugins CI deploy key — generated $(date -u +%Y-%m-%d)"
```

1. Open `https://github.com/Chorosyne/strivo-plugins/settings/keys`.
2. **Add deploy key** → paste `strivo-plugins-deploy.pub`. Leave
   *Allow write access* **off**.
3. Open `https://github.com/Chorosyne/strivo/settings/secrets/actions`.
4. **New repository secret** named `STRIVO_PLUGINS_DEPLOY_KEY`, value
   is the **private** key (`cat strivo-plugins-deploy`).
5. Securely delete the local copies once both are uploaded:
   `shred -u strivo-plugins-deploy strivo-plugins-deploy.pub`.

## 3. CI is already wired

Both `.github/workflows/ci.yml` and `release.yml` pass
`ssh-key: ${{ secrets.STRIVO_PLUGINS_DEPLOY_KEY }}` to
`actions/checkout`. When the secret is unset (forks, public PRs),
checkout falls back to `GITHUB_TOKEN`, which 404s on the private
submodule — the build then proceeds with the empty submodule
directory, and downstream consumers either build successfully without
the plugins or fail with a clear "submodule not present" message
(future hardening: add a `plugins-stub` feature gate).

## 4. Smoke-test from a clean checkout

```bash
# As the runner user, with the deploy key in ssh-agent:
git clone --recurse-submodules git@github.com:Chorosyne/strivo.git /tmp/strivo-test
cd /tmp/strivo-test && cargo build
```

A successful build means the key is wired and the runner can resolve
the private submodule.

## 5. Local development

Developers with access just need their personal GitHub SSH key listed
on `Chorosyne/strivo-plugins`'s collaborator list. No deploy key
needed locally — the submodule URL stays at the HTTPS form in
`.gitmodules` and Git transparently falls through to whichever auth
is configured (SSH agent for personal keys, deploy key for CI).

## 6. Build modes

`strivo-plugins` is consumed as an **optional** Cargo dependency in
`strivo-bin` and `strivo-web`, gated by a `pro` feature that is
**on by default**.

```bash
# Pro contributor (submodule present, default flow)
cargo build

# Free / public-fork build (no submodule access required)
cargo build --no-default-features
```

The free build:
- compiles without ever resolving the `strivo-plugins` git dep,
- ships every other surface (recording, monitor, UI, licence client,
  upgrade card, all 13 Settings sub-sections),
- presents `/plugins` as an empty hub with the upgrade card,
- has Pro plugin data routes return 404 (the module isn't mounted).

The runtime licence gate still applies on top, so even Pro builds
distributed publicly remain locked until activated.

### Picking up local submodule edits without a push

Cargo evaluates `[patch]` tables eagerly, so embedding the patch
in the workspace `Cargo.toml` would break free clones (which
don't have `strivo-plugins/`). Instead, contributors who want
local edits to flow through without round-tripping GitHub drop
this into a gitignored `.cargo/config.toml`:

```toml
[patch."https://github.com/Chorosyne/strivo-plugins"]
strivo-plugins = { path = "strivo-plugins" }
```

`.cargo/` is already in the repo's `.gitignore`.
