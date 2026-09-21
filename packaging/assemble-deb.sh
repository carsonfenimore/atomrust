#!/bin/bash
# Lays out the package tree and builds the .deb. Runs inside the cross container
# (needs dpkg-shlibdeps with the arm64 libraries installed).
set -euo pipefail

BIN=$1
OUT=$2
VERSION=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -1)
PKG=$(mktemp -d)
chmod 755 "$PKG"
trap 'rm -rf "$PKG" debian' EXIT

install -Dm755 "$BIN"                          "$PKG/usr/bin/atomrust"
# In /usr/bin so `atomrust-overlay status` works without sudo; ro/rw check for root.
install -Dm755 packaging/atomrust-overlay      "$PKG/usr/bin/atomrust-overlay"
ln -s atomrust-overlay                         "$PKG/usr/bin/atomrust-ro"
ln -s atomrust-overlay                         "$PKG/usr/bin/atomrust-rw"
install -Dm644 packaging/atomrust.service      "$PKG/usr/lib/systemd/system/atomrust.service"
install -Dm644 packaging/config.yml            "$PKG/etc/atomrust/config.yml"
install -Dm644 models/coco_labels.txt          "$PKG/usr/share/atomrust/models/coco_labels.txt"
install -Dm644 models/ssd_mobilenet_v2_coco_quant_postprocess.tflite \
                                               "$PKG/usr/share/atomrust/models/ssd_mobilenet_v2_coco_quant_postprocess.tflite"
install -Dm644 README.md                       "$PKG/usr/share/doc/atomrust/README.md"
install -Dm644 LICENSE                         "$PKG/usr/share/doc/atomrust/copyright"

# Compute shared-library Depends from the arm64 packages in the container.
mkdir -p debian && printf 'Source: atomrust\n\nPackage: atomrust\nArchitecture: arm64\n' > debian/control
SHLIBS=$(dpkg-shlibdeps -O --ignore-missing-info -xlibc6-dev "$PKG/usr/bin/atomrust" 2>/dev/null \
	| sed -n 's/^shlibs:Depends=//p')
[ -n "$SHLIBS" ] || { echo "dpkg-shlibdeps produced no dependencies" >&2; exit 1; }

# rpicam-apps changes its C++ API between minor releases while keeping the
# librpicam_app.so.1 soname, so shlibdeps' ">= built-against" isn't enough.
# Cap it below the next minor version: apt then holds rpicam-apps back until
# atomrust is rebuilt against the new release, instead of breaking at runtime.
RPICAM_VER=$(dpkg-query -W -f='${Version}' librpicam-app1:arm64)
RPICAM_NEXT=$(echo "$RPICAM_VER" | awk -F'[.-]' '{ printf "%d.%d", $1, $2 + 1 }')
SHLIBS="$SHLIBS, librpicam-app1 (<< ${RPICAM_NEXT}~)"
echo "rpicam-apps: built against $RPICAM_VER; package requires < $RPICAM_NEXT"

install -d "$PKG/DEBIAN"
sed -e "s/@VERSION@/$VERSION/" -e "s/@SHLIBS@/$SHLIBS/" -e "s/@SIZE@/$(du -sk "$PKG" | cut -f1)/" \
	packaging/debian/control.in > "$PKG/DEBIAN/control"
install -m644 packaging/debian/conffiles "$PKG/DEBIAN/conffiles"
install -m755 packaging/debian/postinst packaging/debian/prerm packaging/debian/postrm "$PKG/DEBIAN/"

mkdir -p "$OUT"
fakeroot dpkg-deb --build -Zxz "$PKG" "$OUT/atomrust_${VERSION}_arm64.deb"
dpkg-deb --info "$OUT/atomrust_${VERSION}_arm64.deb" | sed -n '/Package:/,/Description/p'
