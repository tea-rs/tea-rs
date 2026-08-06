#!/usr/bin/env sh
set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <empty-destination-directory>" >&2
  exit 2
fi

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd -P)
source_root=$(CDPATH= cd "$script_dir/.." && pwd -P)
destination=$1

case "$destination" in
  /*) destination_abs=$destination ;;
  *) destination_abs=$(CDPATH= cd "$PWD" && pwd)/$destination ;;
esac

# Resolve the destination before checking whether it is inside the source
# checkout. The destination itself may not exist yet, so resolve its nearest
# existing parent and keep the remaining path components.
if [ -d "$destination_abs" ]; then
  destination_abs=$(CDPATH= cd "$destination_abs" && pwd -P)
else
  destination_parent=$(dirname "$destination_abs")
  destination_name=$(basename "$destination_abs")
  while [ ! -d "$destination_parent" ]; do
    [ "$destination_parent" != "/" ] || {
      echo "destination parent does not exist: $destination_parent" >&2
      exit 1
    }
    destination_name="$(basename "$destination_parent")/$destination_name"
    destination_parent=$(dirname "$destination_parent")
  done
  destination_abs="$(CDPATH= cd "$destination_parent" && pwd -P)/$destination_name"
fi

case "$destination_abs" in
  "$source_root"|"$source_root"/*)
    echo "destination must be outside the source checkout" >&2
    exit 1
    ;;
esac

if [ -e "$destination_abs" ]; then
  [ -d "$destination_abs" ] || {
    echo "destination is not a directory: $destination_abs" >&2
    exit 1
  }
  [ -z "$(find "$destination_abs" -mindepth 1 -maxdepth 1 -print -quit)" ] || {
    echo "destination must be empty: $destination_abs" >&2
    exit 1
  }
else
  mkdir -p "$destination_abs"
fi

copy_path() {
  path=$1
  [ -e "$source_root/$path" ] || {
    echo "public path is missing: $path" >&2
    exit 1
  }
  mkdir -p "$destination_abs/$(dirname "$path")"
  cp -R "$source_root/$path" "$destination_abs/$(dirname "$path")/"
}

copy_file_as() {
  source_path=$1
  destination_path=$2
  [ -f "$source_root/$source_path" ] || {
    echo "public file is missing: $source_path" >&2
    exit 1
  }
  mkdir -p "$destination_abs/$(dirname "$destination_path")"
  cp "$source_root/$source_path" "$destination_abs/$destination_path"
}

# This is an allowlist. Private plans, decisions, audits, traces, and release
# workflows are intentionally absent even when they exist in the source tree.
for path in \
  .cargo \
  .github/actions \
  .github/ISSUE_TEMPLATE \
  .github/workflows/ci.yml \
  .github/workflows/fuzz.yml \
  .github/pull_request_template.md \
  crates \
  fuzz \
  scripts/ci-check.sh \
  scripts/package-cli.sh \
  scripts/public-gitignore \
  scripts/prepare-public-repo.sh \
  scripts/test-cli-platform.sh \
  scripts/tests \
  .env.example \
  Cargo.lock \
  Cargo.toml \
  CONTRIBUTING.md \
  deny.toml \
  LICENSE \
  README.md \
  README.de.md \
  README.es.md \
  README.fr.md \
  README.ja.md \
  README.ko.md \
  README.ru.md \
  README.zh-CN.md \
  README.zh-Hant.md \
  rust-toolchain.toml \
  rustfmt.toml \
  SECURITY.md
do
  copy_path "$path"
done

copy_file_as scripts/public-gitignore .gitignore

git init -b main "$destination_abs" >/dev/null
echo "Prepared a public checkout with no commits: $destination_abs"
echo "Review the allowlisted files, then run:"
echo "  git -C \"$destination_abs\" add -A"
echo "  git -C \"$destination_abs\" commit -m \"Initial public release\""
