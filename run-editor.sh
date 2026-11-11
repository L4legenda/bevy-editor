#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

if ! command -v cargo >/dev/null 2>&1; then
    printf '%s\n' 'Ошибка: Cargo не найден. Установите Rust 1.97.1+ и добавьте Cargo в PATH.' >&2
    exit 127
fi

exec cargo run --locked \
    --manifest-path "$script_dir/Cargo.toml" \
    --bin bevy-editor -- "$@"
