#!/usr/bin/env bash
# Guard against the auto-run malware pattern removed in #520/#521: editor
# configs that execute on folder open, payloads disguised as font/binary files,
# and npm lifecycle hooks that run on install.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "Checking repository hygiene..."

failed=0
fail() {
    echo "Error: $1"
    failed=1
}

tracked="$(git ls-files)"

# Editor/IDE/devcontainer configs can execute commands when a folder is opened.
if hits="$(grep -E '(^|/)\.(vscode|cursor|idea|devcontainer)/' <<<"$tracked")"; then
    fail "editor or devcontainer config must not be committed:"
    sed 's/^/  /' <<<"$hits"
fi

# Fonts and native binaries have no place in this repo and are a common
# hiding spot for payloads.
if hits="$(grep -iE '\.(woff2?|eot|ttf|otf|llf|exe|dll|so|dylib|bat|cmd|ps1|scr|jar)$' <<<"$tracked")"; then
    fail "font or binary files must not be committed:"
    sed 's/^/  /' <<<"$hits"
fi

# npm lifecycle hooks run arbitrary code on `npm install`.
while IFS= read -r manifest; do
    if hooks="$(grep -nE '"(preinstall|install|postinstall|prepare|prepublish|prepack|postpack)"[[:space:]]*:' "$manifest")"; then
        fail "npm lifecycle script in $manifest:"
        sed 's/^/  /' <<<"$hooks"
    fi
done < <(grep -E '(^|/)package\.json$' <<<"$tracked")

# Settings that let tasks run without a prompt, in any config file.
if hits="$(git grep -nIE 'allowAutomaticTasks|"runOn"[[:space:]]*:[[:space:]]*"folderOpen"' -- '*.json' '*.jsonc' '*.code-workspace' || true)" && [ -n "$hits" ]; then
    fail "auto-run task settings found:"
    sed 's/^/  /' <<<"$hits"
fi

if [ "$failed" -ne 0 ]; then
    echo ""
    echo "Repository hygiene check failed. See SECURITY.md for why these are blocked."
    exit 1
fi

echo "Repository hygiene check passed."
