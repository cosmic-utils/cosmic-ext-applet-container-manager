name := 'cosmic-ext-applet-container-manager'
appid := 'org.cosmic_utils.CosmicExtAppletContainerManager'
rootdir := ''
prefix := '/usr'
base-dir := absolute_path(clean(rootdir / prefix))
target-dir := env('CARGO_TARGET_DIR', 'target')
flatpak-manifest := 'flatpak' / appid + '.json'
flatpak-cache-dir := env('HOME') / '.cache' / name
flatpak-build-dir := flatpak-cache-dir / 'build'
flatpak-state-dir := flatpak-cache-dir / 'state'

appdata-dst := base-dir / 'share/metainfo' / appid + '.metainfo.xml'
desktop-dst := base-dir / 'share/applications' / appid + '.desktop'
bin-dst := base-dir / 'bin' / name
icon-dst := base-dir / 'share/icons/hicolor/scalable/apps/org.cosmic_utils.CosmicExtAppletContainerManager-symbolic.svg'

default: build-release

build-debug *args:
    cargo build --locked {{ args }}

build-release *args:
    cargo build --release --locked {{ args }}

fmt-check:
    cargo fmt --all -- --check

cargo-check *args:
    cargo check --locked --all-targets --all-features {{ args }}

test *args:
    cargo test --locked --all-targets --all-features {{ args }}

clippy *args:
    cargo clippy --locked --all-targets --all-features {{ args }} -- -D warnings

check: fmt-check cargo-check test clippy

ci: check

validate-metadata:
    desktop-file-validate resources/app.desktop
    appstreamcli validate --no-net resources/app.metainfo.xml

validate-generated-metadata:
    test -f target/xdgen/app.desktop -a -f target/xdgen/app.metainfo.xml
    desktop-file-validate target/xdgen/app.desktop
    appstreamcli validate --no-net target/xdgen/app.metainfo.xml

run:
    cargo run --release --locked

install: build-release
    install -Dm0755 {{ target-dir / 'release' / name }} {{ bin-dst }}
    install -Dm0644 target/xdgen/app.desktop {{ desktop-dst }}
    install -Dm0644 target/xdgen/app.metainfo.xml {{ appdata-dst }}
    install -Dm0644 resources/icon-symbolic.svg {{ icon-dst }}

uninstall:
    rm -f {{ bin-dst }} {{ desktop-dst }} {{ appdata-dst }} {{ icon-dst }}

flatpak-build:
    flatpak run org.flatpak.Builder --force-clean --user --install-deps-from=flathub --state-dir='{{ flatpak-state-dir }}' '{{ flatpak-build-dir }}' '{{ flatpak-manifest }}'

flatpak-install:
    flatpak run org.flatpak.Builder --force-clean --user --install --install-deps-from=flathub --state-dir='{{ flatpak-state-dir }}' '{{ flatpak-build-dir }}' '{{ flatpak-manifest }}'

flatpak-uninstall:
    flatpak uninstall --user --noninteractive {{ appid }}

setup-screenshot-demo:
    scripts/setup-screenshot-demo.sh
