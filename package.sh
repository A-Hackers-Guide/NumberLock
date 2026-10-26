#!/bin/bash
# Builds NumberLock for macOS (universal), Windows (x64) and Linux (x64 AppImage) from a Mac.
# Needs: rustup targets aarch64/x86_64-apple-darwin, x86_64-pc-windows-gnu, x86_64-unknown-linux-gnu;
#        brew install mingw-w64 zig squashfs; cargo install cargo-zigbuild
set -euo pipefail
cd "$(dirname "$0")"
export PATH=/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH

for t in aarch64-apple-darwin x86_64-apple-darwin x86_64-pc-windows-gnu; do cargo build --release --target $t; done
cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.28

OUT=../releases-native
rm -rf "$OUT" pkg && mkdir -p "$OUT" pkg

# ---- macOS .app
APP=pkg/NumberLock.app
mkdir -p $APP/Contents/MacOS $APP/Contents/Resources pkg/AppIcon.iconset
lipo -create target/{aarch64,x86_64}-apple-darwin/release/numberlock -output $APP/Contents/MacOS/NumberLock
for s in 16 32 128 256 512; do
  cp assets/icon-$s.png pkg/AppIcon.iconset/icon_${s}x${s}.png
  cp assets/icon-$((s * 2)).png pkg/AppIcon.iconset/icon_${s}x${s}@2x.png
done
iconutil -c icns pkg/AppIcon.iconset -o $APP/Contents/Resources/AppIcon.icns
cat > $APP/Contents/Info.plist <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>NumberLock</string>
  <key>CFBundleDisplayName</key><string>NumberLock</string>
  <key>CFBundleIdentifier</key><string>app.numberlock</string>
  <key>CFBundleExecutable</key><string>NumberLock</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>1.0.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
EOF
codesign --force --deep -s - $APP
ditto -c -k --keepParent $APP "$OUT/NumberLock-Mac.zip"

# ---- Windows .exe (single portable file)
cp target/x86_64-pc-windows-gnu/release/numberlock.exe "$OUT/NumberLock-Windows.exe"

# ---- Linux AppImage = AppImage runtime + squashfs of the AppDir
D=pkg/NumberLock.AppDir
mkdir -p $D/usr/bin
cp target/x86_64-unknown-linux-gnu/release/numberlock $D/usr/bin/numberlock
cp assets/icon-256.png $D/numberlock.png
cp assets/icon-256.png $D/.DirIcon
cat > $D/numberlock.desktop <<'EOF'
[Desktop Entry]
Type=Application
Name=NumberLock
Comment=Turn files and folders into password-locked numbers, and back
Exec=numberlock
Icon=numberlock
Categories=Utility;
EOF
printf '#!/bin/sh\nHERE="$(dirname "$(readlink -f "$0")")"\nexec "$HERE/usr/bin/numberlock" "$@"\n' > $D/AppRun
chmod 755 $D/AppRun $D/usr/bin/numberlock
[ -f pkg-cache/runtime-x86_64 ] || { mkdir -p pkg-cache; curl -sSLf -o pkg-cache/runtime-x86_64 \
  https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64; }
mksquashfs $D pkg/app.squashfs -root-owned -noappend -no-xattrs -comp zstd -quiet
cat pkg-cache/runtime-x86_64 pkg/app.squashfs > "$OUT/NumberLock-Linux.AppImage"
chmod 755 "$OUT/NumberLock-Linux.AppImage"

ls -lh "$OUT"
