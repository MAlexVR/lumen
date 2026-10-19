#!/usr/bin/env bash
# Prints the CHANGELOG.md section of one version (without its heading).
# Usage: scripts/changelog-section.sh 0.3.2
#   git tag -a v0.3.2 --cleanup=verbatim -F <(scripts/changelog-section.sh 0.3.2)
set -euo pipefail
version="${1:?usage: changelog-section.sh <version>}"
file="$(dirname "$0")/../CHANGELOG.md"
awk -v v="$version" '
  $0 ~ "^## \\[" v "\\]" { found = 1; next }
  /^## \[/ { found = 0 }
  found
' "$file" | sed '1{/^$/d}' | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}'
