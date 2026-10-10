#!/usr/bin/env bash
#
# Check what the built bigame-mode package installs: every file Big Game Mode
# needs at runtime, with sane modes and owners, and the references between
# them (desktop entry -> binary and icon, D-Bus activation -> systemd unit ->
# binary, bus policy -> bus name, AppStream -> desktop entry, Polkit policy ->
# its source with the translations merged in).
#
#   .github/scripts/check-package-contents.sh <package.pkg.tar.zst> <new-directory>
#
# The package is unpacked into <new-directory>, which stays for the desktop
# entry and AppStream checks that follow. Needs bsdtar and xmllint; never
# installs or runs anything from the package.
set -Eeuo pipefail

package="${1:?usage: $0 <package> <new-directory>}"
root="${2:?usage: $0 <package> <new-directory>}"
repo="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"

app_id=com.biglinux.BiGameMode
domain=bigame-mode

if [[ -e "$root" ]]; then
    echo "error: $root already exists" >&2
    exit 1
fi
mkdir -p -- "$root"
bsdtar -xpf "$package" -C "$root"

failures=0
ok()   { echo "  ok    $*"; }
fail() { echo "  FAIL  $*"; failures=$((failures + 1)); }

# A regular, non-empty file with exactly this mode.
want_file() {
    local path="$1" mode="$2" actual
    if [[ ! -f "$root/$path" || -L "$root/$path" ]]; then
        fail "/$path is missing or not a regular file"
    elif [[ ! -s "$root/$path" ]]; then
        fail "/$path is empty"
    elif actual="$(stat -c %a -- "$root/$path")"; [[ "$actual" != "$mode" ]]; then
        fail "/$path has mode $actual, expected $mode"
    else
        ok "/$path ($mode)"
    fi
}

want_dir() {
    if [[ -d "$root/$1" && ! -L "$root/$1" ]]; then
        ok "/$1/"
    else
        fail "/$1/ is missing"
    fi
}

want_same() {
    local label="$1" actual="$2" expected="$3"
    if [[ "$actual" == "$expected" ]]; then
        ok "$label: $actual"
    else
        fail "$label is '$actual', expected '$expected'"
    fi
}

# An absolute path that is an executable file in the package. Its mode bits,
# not test -x: that asks the kernel, which says no on a noexec mount (/tmp).
want_executable() {
    local label="$1" path="$2"
    if [[ "$path" == /* && -f "$root$path" ]] && (( 8#$(stat -c %a -- "$root$path") & 8#111 )); then
        ok "$label -> $path"
    else
        fail "$label '$path' is not an executable in the package"
    fi
}

# The value of Key= in a desktop-style file (first occurrence).
ini_value() { sed -n "s/^$2=//p" "$1" | head -n 1; }

echo "Installed files:"
want_file usr/bin/bigame-ui 755
want_file usr/bin/bigame-daemon 755
want_file usr/lib/systemd/system/bigame-daemon.service 644
want_file "usr/share/dbus-1/system-services/${app_id}.service" 644
want_file "usr/share/dbus-1/system.d/${app_id}.conf" 644
want_file "usr/share/polkit-1/actions/${app_id}.policy" 644
want_file "usr/share/applications/${app_id}.desktop" 644
want_file "usr/share/metainfo/${app_id}.metainfo.xml" 644
want_file "usr/share/icons/hicolor/scalable/apps/${app_id}.svg" 644
want_file usr/share/icons/hicolor/scalable/apps/bigamemode-symbolic.svg 644
want_file "usr/share/doc/${domain}/README.md" 644
want_file "usr/share/licenses/${domain}/LICENSE" 644
want_dir etc/falcond
want_dir usr/share/falcond/profiles/user
want_file .INSTALL 644

echo "Translations (one catalogue per locale/LINGUAS entry):"
mapfile -t linguas < <(sed -e 's/#.*//' -e '/^[[:space:]]*$/d' "$repo/locale/LINGUAS" | tr -s '[:space:]' '\n' | sed '/^$/d')
if (( ${#linguas[@]} == 0 )); then
    fail "locale/LINGUAS lists no language"
fi
for lang in "${linguas[@]}"; do
    mo="usr/share/locale/${lang}/LC_MESSAGES/${domain}.mo"
    [[ -s "$root/$mo" ]] || fail "/$mo is missing"
done
shipped="$(find "$root/usr/share/locale" -name "${domain}.mo" 2>/dev/null | wc -l)"
if (( shipped == ${#linguas[@]} )); then
    ok "${shipped} catalogues, as listed in locale/LINGUAS"
else
    fail "${shipped} catalogues shipped, ${#linguas[@]} listed in locale/LINGUAS"
fi

echo "Package layout:"
mapfile -t top < <(find "$root" -mindepth 1 -maxdepth 1 ! -name '.*' -printf '%f\n' | sort)
if [[ "${top[*]}" == "etc usr" ]]; then
    ok "installs only under /etc and /usr"
else
    fail "unexpected top-level entries: ${top[*]}"
fi
if [[ -e "$root/usr/local" || -e "$root/usr/lib/debug" || -e "$root/usr/src/debug" ]]; then
    fail "ships /usr/local or debug files (debug files belong in ${domain}-debug)"
else
    ok "no /usr/local, no debug files"
fi
not_root="$(bsdtar -tvf "$package" | awk '$3 != "root" || $4 != "root"')"
if [[ -z "$not_root" ]]; then
    ok "every file owned by root:root"
else
    fail "files not owned by root:root:"
    printf '        %s\n' "$not_root"
fi

echo "References between the files:"
desktop="$root/usr/share/applications/${app_id}.desktop"
dbus_service="$root/usr/share/dbus-1/system-services/${app_id}.service"
unit="$root/usr/lib/systemd/system/bigame-daemon.service"
bus_policy="$root/usr/share/dbus-1/system.d/${app_id}.conf"
metainfo="$root/usr/share/metainfo/${app_id}.metainfo.xml"

if [[ -f "$desktop" ]]; then
    exec_bin="$(ini_value "$desktop" Exec | cut -d ' ' -f 1)"
    [[ "$exec_bin" == /* ]] || exec_bin="/usr/bin/${exec_bin}"
    want_executable "desktop Exec" "$exec_bin"
    icon="$(ini_value "$desktop" Icon)"
    if compgen -G "$root/usr/share/icons/hicolor/*/apps/${icon}.*" >/dev/null; then
        ok "desktop Icon -> $icon"
    else
        fail "desktop Icon $icon is not in the package"
    fi
fi

if [[ -f "$dbus_service" && -f "$unit" ]]; then
    want_same "D-Bus activation Name" "$(ini_value "$dbus_service" Name)" "$app_id"
    want_executable "D-Bus activation Exec" "$(ini_value "$dbus_service" Exec | cut -d ' ' -f 1)"
    want_same "D-Bus activation SystemdService" "$(ini_value "$dbus_service" SystemdService)" "$(basename -- "$unit")"
    want_executable "unit ExecStart" "$(ini_value "$unit" ExecStart | cut -d ' ' -f 1)"
    want_same "unit BusName" "$(ini_value "$unit" BusName)" "$app_id"
fi

if [[ -f "$bus_policy" ]]; then
    if grep -q "<allow own=\"${app_id}\"/>" "$bus_policy"; then
        ok "bus policy lets the helper own $app_id"
    else
        fail "bus policy lets no one own $app_id"
    fi
fi

polkit_policy="$root/usr/share/polkit-1/actions/${app_id}.policy"
if [[ -f "$polkit_policy" ]]; then
    # The installed policy is the source with the catalogues merged in: the
    # same actions, each with its English description and message as written.
    source_policy="$repo/data/${app_id}.policy"
    if [[ "$(grep -o '<action id="[^"]*"' "$polkit_policy")" == "$(grep -o '<action id="[^"]*"' "$source_policy")" ]]; then
        ok "Polkit policy has the actions of data/${app_id}.policy"
    else
        fail "Polkit policy's actions differ from data/${app_id}.policy"
    fi
    if [[ "$(grep -E '<(description|message)>' "$polkit_policy")" == "$(grep -E '<(description|message)>' "$source_policy")" ]]; then
        ok "Polkit policy keeps every English description and message"
    else
        fail "Polkit policy's English descriptions or messages differ from the source"
    fi
    # polkit matches pt_BR, never msgfmt's pt-BR.
    if grep -qE 'xml:lang="[a-z]{2,3}-[A-Z]{2}"' "$polkit_policy"; then
        fail "Polkit policy has a translation tagged ll-CC, which polkit never serves"
    else
        ok "Polkit policy's translations are tagged as polkit looks them up"
    fi
fi

if [[ -f "$metainfo" ]]; then
    launchable="$(sed -n 's:.*<launchable type="desktop-id">\(.*\)</launchable>.*:\1:p' "$metainfo")"
    want_same "AppStream launchable" "$launchable" "${app_id}.desktop"
fi

echo "XML well-formedness:"
for xml in "$bus_policy" "$root/usr/share/polkit-1/actions/${app_id}.policy" "$metainfo"; do
    [[ -f "$xml" ]] || continue
    if xmllint --noout --nonet "$xml"; then ok "${xml#"$root"}"; else fail "${xml#"$root"} is not well-formed XML"; fi
done

echo
if (( failures )); then
    echo "${failures} package content check(s) failed"
    exit 1
fi
echo "Package contents OK"
