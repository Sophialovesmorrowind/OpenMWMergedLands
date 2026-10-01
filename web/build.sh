#!/bin/sh
set -eu
cd "$(dirname "$0")"
PATH="$PWD/.tools:$PATH"
export PATH

if ! command -v cmark >/dev/null 2>&1; then
    echo 'Install cmark (the CommonMark renderer) to build the Markdown pages.' >&2
    exit 1
fi

# Adapted from MOMW Mod Template. See site/licenses/MOMW-Mod-Template-MIT.txt.
if [ -z "${SOUPAULT:-}" ]; then
    SOUPAULT=.tools/soupault-4.8.0-linux-x86_64/soupault
    if [ ! -x "$SOUPAULT" ]; then
        if [ "$(uname -s)-$(uname -m)" != Linux-x86_64 ]; then
            echo 'Install Soupault 4.8.0 and set SOUPAULT to its executable path.' >&2
            exit 1
        fi
        mkdir -p .tools
        archive=.tools/soupault-4.8.0-linux-x86_64.tar.gz
        curl --fail --show-error --silent --location --retry 3 \
            https://github.com/PataphysicalSociety/soupault/releases/download/4.8.0/soupault-4.8.0-linux-x86_64.tar.gz \
            --output "$archive"
        printf '%s  %s\n' dd8ecc792958d88cb217bdf87825626925ea1b0b893311a0662aa99820697974 "$archive" | sha256sum --check -
        tar -xzf "$archive" -C .tools
    fi
fi
# Discard generated pages from older layouts, including the former .html routes.
python3 -c 'import shutil; shutil.rmtree("build", ignore_errors=True)'
"$SOUPAULT" "$@"
