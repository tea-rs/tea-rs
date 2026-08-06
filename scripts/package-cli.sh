#!/usr/bin/env sh
set -eu

host_target=$(rustc -vV | sed -n 's/^host: //p')
package_target=${TEA_PACKAGE_TARGET:-$host_target}
package_profile=${TEA_PACKAGE_PROFILE:-all}
package_dir=${TEA_PACKAGE_DIR:-target/dist}
cargo_target_dir=${CARGO_TARGET_DIR:-target}

case "$package_target" in
  x86_64-unknown-linux-gnu|aarch64-apple-darwin)
    ;;
  *)
    echo "unsupported CLI package target: $package_target" >&2
    echo "selected targets: x86_64-unknown-linux-gnu, aarch64-apple-darwin" >&2
    exit 2
    ;;
esac

case "$package_profile" in
  debug|release|all)
    ;;
  *)
    echo "TEA_PACKAGE_PROFILE must be debug, release, or all" >&2
    exit 2
    ;;
esac

package_id=$(cargo pkgid -p tea-cli)
package_version=${package_id##*#}
if [ -z "$package_version" ] || [ "$package_version" = "$package_id" ]; then
  echo "unable to resolve tea-cli package version" >&2
  exit 1
fi

mkdir -p "$package_dir"
temporary_dir=$(mktemp -d "${TMPDIR:-/tmp}/tea-cli-package.XXXXXX")
cleanup() {
  rm -r "$temporary_dir"
}
trap cleanup EXIT HUP INT TERM

checksum() {
  archive_name=$1
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$package_dir" && sha256sum "$archive_name" >"$archive_name.sha256")
  elif command -v shasum >/dev/null 2>&1; then
    (cd "$package_dir" && shasum -a 256 "$archive_name" >"$archive_name.sha256")
  else
    echo "sha256sum or shasum is required" >&2
    exit 1
  fi
}

build_bundle() {
  profile=$1
  cargo_profile=$profile
  release_flag=
  if [ "$profile" = release ]; then
    release_flag=--release
  fi

  cargo build --locked --target "$package_target" -p tea-cli $release_flag

  source_binary="$cargo_target_dir/$package_target/$cargo_profile/tea"
  if [ ! -x "$source_binary" ]; then
    echo "built tea binary is missing: $source_binary" >&2
    exit 1
  fi

  bundle_name="tea-$package_version-$package_target-$profile"
  bundle_root="$temporary_dir/$bundle_name"
  mkdir -p "$bundle_root"
  cp "$source_binary" "$bundle_root/tea"
  chmod 755 "$bundle_root/tea"
  cp LICENSE "$bundle_root/LICENSE"
  cp crates/tea-cli/README.md "$bundle_root/README.md"

  archive_name="$bundle_name.tar.gz"
  tar -czf "$package_dir/$archive_name" -C "$temporary_dir" "$bundle_name"
  checksum "$archive_name"
  echo "CLI package: $package_dir/$archive_name"
  echo "CLI checksum: $package_dir/$archive_name.sha256"
}

case "$package_profile" in
  debug)
    build_bundle debug
    ;;
  release)
    build_bundle release
    ;;
  all)
    build_bundle debug
    build_bundle release
    ;;
esac
