# syntax=docker/dockerfile:1-labs

# For all stages
ARG CRATE_NAME=edgehog-device-runtime
ARG CARGO_TARGET_DIR=/build/target

FROM --platform=$BUILDPLATFORM rust AS builder

ARG CRATE_NAME
ARG CARGO_TARGET_DIR
ARG TARGETOS
ARG TARGETARCH

WORKDIR /build/$CRATE_NAME

RUN set -eux && \
    cargo install --locked bindgen-cli && \
    apt-get update && \
    apt-get install -y mingw-w64 build-essential clang cmake

RUN cargo init

COPY --parents ./Cargo.lock ./Cargo.toml ./*/Cargo.toml ./*/*/main.rs ./*/*/lib.rs /build/$CRATE_NAME
RUN cargo fetch

# TODO: this can be more restrictive
COPY . /build/$CRATE_NAME

RUN set -eux && \
    export TARGET_ARCH="$(echo $TARGETARCH | sed -e 's/arm64/aarch64/' -e 's/amd64/x86_64/')" &&\
    export TARGET_TRIPPLET="$TARGET_ARCH-pc-windows-gnu" &&\
    export ENV_TRIPPLET=$(echo $TARGET_TRIPPLET | sed -e 's/-/_/g' | tr '[:lower:]' '[:upper:]') &&\
    export "CARGO_TARGET_${ENV_TRIPPLET}_LINKER"="x86_64-w64-mingw32-gcc" &&\
    export RUSTCFLAGS="-C target-feature=+crt-static" &&\
    rustup target add "$TARGET_TRIPPLET" &&\
    cargo build --release --target="$TARGET_TRIPPLET" --no-default-features --features=cross,containers,forwarder,file-transfer &&\
    cp "$CARGO_TARGET_DIR/$TARGET_TRIPPLET/release/$CRATE_NAME.exe" /

FROM scratch

ARG CRATE_NAME
ARG CARGO_TARGET_DIR

COPY --from=builder /$CRATE_NAME.exe /$CRATE_NAME.exe

ENTRYPOINT ["/edgehog-device-runtime.exe"]
