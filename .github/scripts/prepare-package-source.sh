#!/usr/bin/env bash
#
# Prepare a makepkg directory that builds the checked-out commit with the
# project's own recipe.
#
# pkgbuild/PKGBUILD fetches main from GitHub (git+https://…), which is what
# the BigLinux package builders want, and wrong for CI: a pull request would
# build main and pass for code it never compiled. Here the recipe and its
# install script are taken as committed in HEAD, HEAD is packed with
# git archive under the directory the recipe builds in, and a source and
# checksum override pointing at that tarball is appended. Nothing else in the
# recipe changes: prepare(), build(), check() and package() run as written.
#
#   .github/scripts/prepare-package-source.sh <empty-or-new-directory>
#
# Committed content only: uncommitted edits are not built. Run it as the user
# that will run makepkg (makepkg refuses root).
set -Eeuo pipefail

out="${1:?usage: $0 <empty-or-new-directory>}"
repo="$(git rev-parse --show-toplevel)"
commit="$(git -C "$repo" rev-parse --verify HEAD)"

if [[ -e "$out" && -n "$(ls -A -- "$out")" ]]; then
    echo "error: $out exists and is not empty" >&2
    exit 1
fi
mkdir -p -- "$out"
out="$(cd -- "$out" && pwd)"

git -C "$repo" archive HEAD:pkgbuild | tar -x -C "$out"

srcinfo() { (cd -- "$out" && makepkg --printsrcinfo); }
original="$(srcinfo)"

mapfile -t pkgnames < <(sed -n 's/^pkgname = //p' <<<"$original")
if (( ${#pkgnames[@]} != 1 )); then
    echo "error: expected one pkgname in the recipe, found: ${pkgnames[*]}" >&2
    exit 1
fi
pkgname="${pkgnames[0]}"

# The override replaces the whole source array. That is only right while the
# recipe has a single source, the project's own repository, unpacked into
# $srcdir/$pkgname. A recipe that gains patches or other sources must not be
# reduced to one tarball silently: stop and have this script updated.
mapfile -t sources < <(grep -E '^[[:space:]]source(_[^ ]+)? = ' <<<"$original" | sed 's/^[[:space:]]*//')
if (( ${#sources[@]} != 1 )) || [[ "${sources[0]}" != "source = ${pkgname}::git+"* ]]; then
    echo "error: the recipe's sources changed; update $0 to match:" >&2
    printf '  %s\n' "${sources[@]}" >&2
    exit 1
fi

tarball="${pkgname}-${commit:0:12}.tar.gz"
git -C "$repo" archive --format=tar.gz --prefix="${pkgname}/" -o "${out}/${tarball}" "$commit"
checksum="$(sha256sum -- "${out}/${tarball}" | cut -d ' ' -f 1)"

cat >> "${out}/PKGBUILD" <<EOF

# --- Added by .github/scripts/prepare-package-source.sh, CI only ---
# Build commit ${commit} from its own tree instead of main from GitHub.
source=('${tarball}')
sha256sums=('${checksum}')
EOF

# What makepkg will now fetch: the tarball and nothing else.
mapfile -t sources < <(srcinfo | grep -E '^[[:space:]]source(_[^ ]+)? = ' | sed 's/^[[:space:]]*//')
if (( ${#sources[@]} != 1 )) || [[ "${sources[0]}" != "source = ${tarball}" ]]; then
    echo "error: the source override did not take effect:" >&2
    printf '  %s\n' "${sources[@]}" >&2
    exit 1
fi

echo "Recipe:  ${out}/PKGBUILD (pkgbuild/PKGBUILD of HEAD, source overridden)"
echo "Commit:  ${commit}"
echo "Source:  ${tarball}"
echo "SHA-256: ${checksum}"
