#!/usr/bin/env bash
# strivo dev install — builds the workspace from this checkout and drops
# `strivo` on your PATH, with every first-party plugin (crunchr, archiver)
# enabled in a default config.
#
# Idempotent: re-running upgrades the binary but never overwrites your
# existing config or shell rc files.
#
# Usage:
#     scripts/install-dev.sh                 # release build (default)
#     scripts/install-dev.sh --debug         # debug build, faster to iterate
#     scripts/install-dev.sh --uninstall     # remove the installed bits
#     scripts/install-dev.sh --reconfigure   # overwrite the generated config
#                                            # block (preserves user sections)
#
# Environment overrides:
#     STRIVO_BIN_DIR     install dir for the binary  (default: ~/.local/bin)
#     STRIVO_SHARE_DIR   support files               (default: ~/.local/share/strivo)
#     STRIVO_CONFIG_DIR  config dir                  (default: ~/.config/strivo)
#     CARGO              cargo binary                (default: cargo)

set -euo pipefail

# ── locate repo root ─────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# ── parse args ───────────────────────────────────────────────────────────────
MODE="release"
ACTION="install"
RECONFIGURE=0

for arg in "$@"; do
    case "$arg" in
        --debug)       MODE="debug" ;;
        --release)     MODE="release" ;;
        --uninstall)   ACTION="uninstall" ;;
        --reconfigure) RECONFIGURE=1 ;;
        -h|--help)
            sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        *)
            echo "✗ unknown argument: $arg" >&2
            echo "  see --help" >&2
            exit 2
            ;;
    esac
done

# ── paths ────────────────────────────────────────────────────────────────────
BIN_DIR="${STRIVO_BIN_DIR:-$HOME/.local/bin}"
SHARE_DIR="${STRIVO_SHARE_DIR:-$HOME/.local/share/strivo}"
CONFIG_DIR="${STRIVO_CONFIG_DIR:-$HOME/.config/strivo}"
CARGO_BIN="${CARGO:-cargo}"
BIN_PATH="$BIN_DIR/strivo"
WHISPERX_SCRIPT_SRC="$REPO_ROOT/strivo-plugins/scripts/whisperx_diarize.py"
WHISPERX_SCRIPT_DEST="$BIN_DIR/whisperx_diarize.py"  # sibling-of-binary
                                                     # — picked up automatically
                                                     # by the whisperx-local
                                                     # backend's discovery rules.
CONFIG_FILE="$CONFIG_DIR/config.toml"
MANAGED_MARKER="# >>> strivo install-dev.sh >>>"
MANAGED_END="# <<< strivo install-dev.sh <<<"

log()  { printf '› %s\n' "$*"; }
warn() { printf '⚠ %s\n' "$*" >&2; }
die()  { printf '✗ %s\n' "$*" >&2; exit 1; }

# ── uninstall path ───────────────────────────────────────────────────────────
if [[ "$ACTION" == "uninstall" ]]; then
    log "removing $BIN_PATH"
    rm -f "$BIN_PATH"
    log "removing $WHISPERX_SCRIPT_DEST"
    rm -f "$WHISPERX_SCRIPT_DEST"
    log "removing $SHARE_DIR/completions"
    rm -rf "$SHARE_DIR/completions"
    log "removing $SHARE_DIR/man"
    rm -rf "$SHARE_DIR/man"
    log "config preserved at $CONFIG_FILE (remove manually if desired)"
    log "✓ uninstalled"
    exit 0
fi

# ── preflight ────────────────────────────────────────────────────────────────
command -v "$CARGO_BIN" >/dev/null 2>&1 \
    || die "cargo not found (set CARGO=/path/to/cargo or install rustup)"

if [[ -f .gitmodules ]] && ! [[ -f "$WHISPERX_SCRIPT_SRC" ]]; then
    log "initializing submodules (strivo-plugins missing)"
    git submodule update --init --recursive
fi

# ── build ────────────────────────────────────────────────────────────────────
if [[ "$MODE" == "release" ]]; then
    log "building strivo-bin (release)"
    "$CARGO_BIN" build --release -p strivo-bin
    BUILT_BIN="$REPO_ROOT/target/release/strivo"
else
    log "building strivo-bin (debug)"
    "$CARGO_BIN" build -p strivo-bin
    BUILT_BIN="$REPO_ROOT/target/debug/strivo"
fi

[[ -x "$BUILT_BIN" ]] || die "build succeeded but $BUILT_BIN is missing"

# ── install binary + sidecar python script ───────────────────────────────────
mkdir -p "$BIN_DIR" "$SHARE_DIR" "$CONFIG_DIR"

log "installing → $BIN_PATH"
install -m 0755 "$BUILT_BIN" "$BIN_PATH"

if [[ -f "$WHISPERX_SCRIPT_SRC" ]]; then
    log "installing → $WHISPERX_SCRIPT_DEST (whisperx orchestrator)"
    install -m 0755 "$WHISPERX_SCRIPT_SRC" "$WHISPERX_SCRIPT_DEST"
else
    warn "whisperx orchestrator missing in submodule; whisperx-local backend"
    warn "will fall back to its CARGO_MANIFEST_DIR lookup or fail with a clear"
    warn "error message if invoked."
fi

# ── completions + manpage ────────────────────────────────────────────────────
log "generating shell completions → $SHARE_DIR/completions"
mkdir -p "$SHARE_DIR/completions"
for shell in bash zsh fish; do
    out="$SHARE_DIR/completions/strivo.$shell"
    if "$BIN_PATH" completions "$shell" > "$out.tmp" 2>/dev/null; then
        mv "$out.tmp" "$out"
    else
        warn "couldn't generate $shell completions (continuing)"
        rm -f "$out.tmp"
    fi
done

log "generating manpage → $SHARE_DIR/man/man1/strivo.1"
mkdir -p "$SHARE_DIR/man/man1"
if "$BIN_PATH" man > "$SHARE_DIR/man/man1/strivo.1.tmp" 2>/dev/null; then
    mv "$SHARE_DIR/man/man1/strivo.1.tmp" "$SHARE_DIR/man/man1/strivo.1"
else
    warn "couldn't generate manpage (continuing)"
    rm -f "$SHARE_DIR/man/man1/strivo.1.tmp"
fi

# ── config: enable every plugin by default ───────────────────────────────────
# We never clobber an existing user config. The first run drops a self-marked
# block; subsequent runs only rewrite that block when --reconfigure is set, so
# any edits to other sections (themes, auto-record channels, etc.) are kept.
write_managed_block() {
    cat <<TOML
$MANAGED_MARKER
# This block was generated by scripts/install-dev.sh. Re-run with
# --reconfigure to refresh it; everything outside the markers is preserved.

[crunchr]
enabled     = true
backend     = "voxtral-openrouter"
api_key_env = "OPENROUTER_API_KEY"
diarize     = false
embed_subs  = true

[crunchr.analysis]
enabled                = false
openrouter_api_key_env = "OPENROUTER_API_KEY"

[archiver]
enabled = true
$MANAGED_END
TOML
}

if [[ ! -f "$CONFIG_FILE" ]]; then
    log "writing default config → $CONFIG_FILE"
    write_managed_block > "$CONFIG_FILE"
elif [[ "$RECONFIGURE" -eq 1 ]]; then
    if grep -qF "$MANAGED_MARKER" "$CONFIG_FILE"; then
        # Marker-bracketed managed block exists — refresh just that span.
        log "refreshing managed config block in $CONFIG_FILE"
        tmp="$(mktemp)"
        awk -v marker="$MANAGED_MARKER" -v end="$MANAGED_END" '
            $0 == marker { skip = 1; next }
            $0 == end    { skip = 0; next }
            !skip        { print }
        ' "$CONFIG_FILE" > "$tmp"
        {
            cat "$tmp"
            echo
            write_managed_block
        } > "$CONFIG_FILE"
        rm -f "$tmp"
    else
        # Pre-existing user config (likely written by `strivo` itself, which
        # owns the section layout). Appending a managed `[crunchr]`/`[archiver]`
        # block here would create duplicate TOML tables, so instead do the
        # minimum surgical edit: flip `enabled = false` → `enabled = true`
        # *inside* the existing `[crunchr]` and `[archiver]` sections. Anything
        # else (backend choice, channel config, etc.) is the user's call.
        log "no marker block found; flipping enabled=true on [crunchr] / [archiver] in place"
        cp "$CONFIG_FILE" "$CONFIG_FILE.bak.$(date +%s)"
        tmp="$(mktemp)"
        awk '
            /^\s*\[[^]]+\]\s*$/ {
                section = $0
                gsub(/^\s*\[|\]\s*$/, "", section)
                print
                next
            }
            (section == "crunchr" || section == "archiver") && /^[[:space:]]*enabled[[:space:]]*=[[:space:]]*false[[:space:]]*$/ {
                sub(/false/, "true")
                print
                next
            }
            { print }
        ' "$CONFIG_FILE" > "$tmp"
        mv "$tmp" "$CONFIG_FILE"
        log "  backup written next to config (config.toml.bak.*); inspect with"
        log "  \`diff $CONFIG_FILE \$(ls -t $CONFIG_FILE.bak.* | head -1)\`"
    fi
else
    log "config exists, leaving alone ($CONFIG_FILE)"
    log "  use --reconfigure to enable [crunchr] / [archiver] in place"
fi

# ── PATH hint ────────────────────────────────────────────────────────────────
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
        warn "$BIN_DIR is not on your PATH"
        warn "add this to your shell rc (.bashrc / .zshrc):"
        warn "    export PATH=\"$BIN_DIR:\$PATH\""
        ;;
esac

# ── completions hint ─────────────────────────────────────────────────────────
shell_name="${SHELL##*/}"
case "$shell_name" in
    bash)
        log "to load bash completions add to ~/.bashrc:"
        log "    source $SHARE_DIR/completions/strivo.bash"
        ;;
    zsh)
        log "to load zsh completions add to ~/.zshrc:"
        log "    fpath=($SHARE_DIR/completions \$fpath)"
        log "    autoload -U compinit && compinit"
        log "(rename strivo.zsh → _strivo if compinit complains)"
        ;;
    fish)
        log "to load fish completions:"
        log "    install -m644 $SHARE_DIR/completions/strivo.fish ~/.config/fish/completions/strivo.fish"
        ;;
esac
log "manpage available via: MANPATH=$SHARE_DIR/man:\$MANPATH man strivo"

# ── final status ─────────────────────────────────────────────────────────────
version="$("$BIN_PATH" --version 2>/dev/null || echo "dev")"
log "✓ installed $version → $BIN_PATH"
log "  config: $CONFIG_FILE"
log "  plugins enabled: crunchr, archiver"
log "  run \`strivo\` to launch the TUI"
log "  optional: \`strivo daemon install\` to register the systemd user unit"
