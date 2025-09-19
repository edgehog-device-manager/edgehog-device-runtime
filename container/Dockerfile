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
    dpkg --add-architecture "$TARGETARCH" && \
    apt-get update && \
    apt-get install --assume-yes build-essential clang cmake "crossbuild-essential-$TARGETARCH"

RUN cargo init

COPY --parents ./Cargo.lock ./Cargo.toml ./*/Cargo.toml ./*/*/main.rs ./*/*/lib.rs /build/$CRATE_NAME
RUN cargo fetch

# TODO: this can be more restrictive
COPY . /build/$CRATE_NAME

RUN set -eux && \
    export TARGET_ARCH="$(echo $TARGETARCH | sed -e 's/arm64/aarch64/' -e 's/amd64/x86_64/')" &&\
    export TARGET_TRIPPLET="$TARGET_ARCH-unknown-linux-gnu" &&\
    export ENV_TRIPPLET=$(echo $TARGET_TRIPPLET | sed -e 's/-/_/g' | tr '[:lower:]' '[:upper:]') &&\
    export "CARGO_TARGET_${ENV_TRIPPLET}_LINKER"="${TARGET_ARCH}-linux-gnu-gcc" &&\
    export RUSTCFLAGS="-C target-feature=+crt-static" &&\
    rustup target add "$TARGET_TRIPPLET" &&\
    cargo build --release --target="$TARGET_TRIPPLET" --no-default-features --features=cross,containers,forwarder

RUN cp "$(find "$CARGO_TARGET_DIR" -name "$CRATE_NAME")" /

FROM scratch

ARG CRATE_NAME
ARG CARGO_TARGET_DIR

COPY --from=builder /$CRATE_NAME /usr/bin/$CRATE_NAME

ENTRYPOINT ["/usr/bin/edgehog-device-runtime"]
