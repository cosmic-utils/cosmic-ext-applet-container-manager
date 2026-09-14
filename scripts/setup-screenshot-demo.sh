#!/bin/sh
set -eu

confirmation_phrase='DELETE ALL CONTAINERS'
demo_label='org.cosmic_utils.container-manager.demo=true'
images='docker.io/library/nginx:alpine
docker.io/jellyfin/jellyfin:latest
ghcr.io/home-assistant/home-assistant:stable
docker.io/library/nextcloud:apache'

tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

skip_docker=false
if command -v docker >/dev/null 2>&1 && command -v podman >/dev/null 2>&1; then
    docker_path=$(command -v docker)
    podman_path=$(command -v podman)
    docker_real=$(readlink -f "$docker_path" 2>/dev/null || printf '%s' "$docker_path")
    podman_real=$(readlink -f "$podman_path" 2>/dev/null || printf '%s' "$podman_path")
    docker_version=$(docker --version 2>/dev/null || true)

    case "$docker_version" in
        *[Pp][Oo][Dd][Mm][Aa][Nn]*) skip_docker=true ;;
    esac
    if [ "$docker_real" = "$podman_real" ]; then
        skip_docker=true
    fi
    if [ "$skip_docker" = true ]; then
        echo 'Docker resolves to Podman; treating both commands as one Podman engine.'
    fi
fi

detected_engines=''
probe_empty_alias=false
for engine in docker podman; do
    if [ "$engine" = docker ] && [ "$skip_docker" = true ]; then
        continue
    fi
    if ! command -v "$engine" >/dev/null 2>&1; then
        continue
    fi

    if "$engine" ps -a --no-trunc --format '{{.ID}}\t{{.Names}}\t{{.Status}}' \
        >"$tmp_dir/$engine.display" 2>"$tmp_dir/$engine.error"; then
        : >"$tmp_dir/$engine.ids"
        tab=$(printf '\t')
        while IFS="$tab" read -r container_id _rest; do
            if [ -n "$container_id" ]; then
                printf '%s\n' "$container_id" >>"$tmp_dir/$engine.ids"
            fi
        done <"$tmp_dir/$engine.display"
        LC_ALL=C sort -u "$tmp_dir/$engine.ids" -o "$tmp_dir/$engine.ids"
        detected_engines="$detected_engines $engine"
    else
        printf 'Skipping %s: it could not be queried successfully.\n' "$engine" >&2
    fi
done

if [ -z "$detected_engines" ]; then
    echo 'No container engine was found and queried successfully.' >&2
    exit 1
fi

if [ "$detected_engines" = ' docker podman' ] &&
    cmp -s "$tmp_dir/docker.ids" "$tmp_dir/podman.ids"; then
    if [ -s "$tmp_dir/docker.ids" ]; then
        echo 'Docker and Podman returned the same container IDs; treating both commands as one Podman engine.'
        detected_engines=' podman'
    else
        probe_empty_alias=true
    fi
fi

cat <<'WARNING'

WARNING: This will permanently force-remove every container listed below.
Images, volumes, networks, and engine configuration will not be removed.
WARNING

for engine in $detected_engines; do
    printf '\n%s containers:\n' "$engine"
    if [ -s "$tmp_dir/$engine.display" ]; then
        while IFS= read -r container; do
            printf '  %s\n' "$container"
        done <"$tmp_dir/$engine.display"
    else
        echo '  (none)'
    fi
done

printf '\nType %s to continue: ' "$confirmation_phrase"
if ! IFS= read -r confirmation || [ "$confirmation" != "$confirmation_phrase" ]; then
    echo 'Cancelled. No containers or images were changed.'
    exit 0
fi

# Pull every required image before deleting anything. A network or registry
# failure therefore leaves the user's existing containers untouched.
for engine in $detected_engines; do
    printf '\nPulling screenshot images with %s...\n' "$engine"
    printf '%s\n' "$images" | while IFS= read -r image; do
        "$engine" pull "$image"
    done
done

# Empty stores have no IDs to compare. After confirmation and successful image
# pulls, use a disposable container to determine whether both clients address
# the same store, then remove it before touching the user's snapshot.
if [ "$probe_empty_alias" = true ]; then
    probe_id=$(docker create \
        --name cosmic-demo-engine-alias-probe \
        --label "$demo_label" \
        docker.io/library/nginx:alpine)
    same_empty_engine=false
    if podman inspect "$probe_id" >/dev/null 2>&1; then
        same_empty_engine=true
    fi
    docker rm -f "$probe_id" >/dev/null
    if [ "$same_empty_engine" = true ]; then
        echo 'Docker and Podman address the same empty store; treating both commands as one Podman engine.'
        detected_engines=' podman'
    fi
fi

# Fail closed if container state changed while the user was reviewing the
# warning or while images were being pulled.
for engine in $detected_engines; do
    "$engine" ps -aq --no-trunc | LC_ALL=C sort -u >"$tmp_dir/$engine.current-ids"
    if ! cmp -s "$tmp_dir/$engine.ids" "$tmp_dir/$engine.current-ids"; then
        printf '%s container state changed; aborting before deletion. Run the command again.\n' \
            "$engine" >&2
        exit 1
    fi
done

for engine in $detected_engines; do
    printf '\nRemoving existing %s containers...\n' "$engine"
    while IFS= read -r container_id; do
        if [ -n "$container_id" ]; then
            "$engine" rm -f "$container_id"
        fi
    done <"$tmp_dir/$engine.ids"

    printf 'Creating %s screenshot containers...\n' "$engine"
    "$engine" run -d \
        --name cosmic-demo-web \
        --label "$demo_label" \
        docker.io/library/nginx:alpine
    "$engine" run -d \
        --name cosmic-demo-jellyfin \
        --label "$demo_label" \
        docker.io/jellyfin/jellyfin:latest
    "$engine" run -d \
        --name cosmic-demo-home-assistant \
        --label "$demo_label" \
        ghcr.io/home-assistant/home-assistant:stable
    "$engine" stop --time 5 cosmic-demo-home-assistant
    "$engine" create \
        --name cosmic-demo-nextcloud \
        --label "$demo_label" \
        docker.io/library/nextcloud:apache

done

printf '\nScreenshot environment ready:\n'
for engine in $detected_engines; do
    printf '\n%s:\n' "$engine"
    "$engine" ps -a --no-trunc --format '  {{.Names}}\t{{.Image}}\t{{.Status}}'
done
