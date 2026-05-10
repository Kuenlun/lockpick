#!/usr/bin/env bash
set -euo pipefail

# Verifies that every source file in $SOURCE_DIRS matching $FILE_GLOB starts
# with the exact contents of $LICENSE_FILE. All three are configurable via
# environment variables so the script can be reused across projects without
# editing it.
#
# Defaults target a Rust crate: the header lives at .github/license_header.rs
# and the script scans src/, tests/, examples/ and benches/ (whichever exist).
# Directories listed in SOURCE_DIRS but absent from the project are silently
# skipped, so the same defaults work for crates that only ship a subset.
LICENSE_FILE="${LICENSE_FILE:-.github/license_header.rs}"
SOURCE_DIRS="${SOURCE_DIRS:-src tests examples benches}"
FILE_GLOB="${FILE_GLOB:-*.rs}"

if [ ! -f "$LICENSE_FILE" ]; then
  echo "ERROR: license header file not found: $LICENSE_FILE"
  echo "Create it with the project's own SPDX/copyright lines, or set"
  echo "LICENSE_FILE to the correct path before running this check."
  exit 1
fi

existing_dirs=()
for dir in $SOURCE_DIRS; do
  if [ -d "$dir" ]; then
    existing_dirs+=( "$dir" )
  fi
done

if [ ${#existing_dirs[@]} -eq 0 ]; then
  echo "ERROR: none of the configured SOURCE_DIRS exist: $SOURCE_DIRS"
  echo "Set SOURCE_DIRS to one or more directories that contain source files."
  exit 1
fi

HEADER_LINES="$(wc -l < "$LICENSE_FILE")"
MISSING=0

while IFS= read -r file; do
  if ! head -n "$HEADER_LINES" "$file" | diff -q - "$LICENSE_FILE" > /dev/null; then
    echo "Missing or incorrect license header in: $file"
    MISSING=1
  fi
done < <(find "${existing_dirs[@]}" -type f -name "$FILE_GLOB")

if [ "$MISSING" -eq 1 ]; then
  echo "Some files are missing the correct license header."
  exit 1
fi

echo "All files have the correct license header."
