#!/bin/sh
set -eu

if [ "$(uname -s)" != "Darwin" ]; then
  echo "macOS 打包需要在 macOS 上运行。" >&2
  exit 1
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
project_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
cd "$project_dir"

npm run build:app

product_name=$(node --input-type=module -e 'import fs from "node:fs"; process.stdout.write(JSON.parse(fs.readFileSync("src-tauri/tauri.conf.json", "utf8")).productName)')
bundle_dir="$project_dir/src-tauri/target/release/bundle"
app_name="$product_name.app"
app_path="$bundle_dir/macos/$app_name"
if [ ! -d "$app_path" ]; then
  echo "找不到已构建的应用：$app_path" >&2
  exit 1
fi
codesign --verify --deep --strict "$app_path"

version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app_path/Contents/Info.plist")
architecture=$(uname -m)
case "$architecture" in
  arm64) architecture=aarch64 ;;
esac
dmg_name="${product_name}_${version}_${architecture}.dmg"
dmg_dir="$bundle_dir/dmg"
mkdir -p "$dmg_dir"

staging_dir=$(mktemp -d "${TMPDIR:-/tmp}/dbm-dmg.XXXXXX")
trap 'rm -rf "$staging_dir"' EXIT
trap 'exit 1' HUP INT TERM
mkdir "$staging_dir/contents"
ditto "$app_path" "$staging_dir/contents/$app_name"
ln -s /Applications "$staging_dir/contents/Applications"

# Native image creation avoids Finder automation and verifies before replacing
# the previous installer. Only the private staging directory is cleaned up.
hdiutil create -volname "$product_name" -srcfolder "$staging_dir/contents" -format UDZO "$staging_dir/$dmg_name"
hdiutil verify "$staging_dir/$dmg_name"
mv -f "$staging_dir/$dmg_name" "$dmg_dir/$dmg_name"
echo "应用：$app_path"
echo "安装包：$dmg_dir/$dmg_name"
