FROM rust:alpine3.20 AS builder

RUN apk add --no-cache build-base openssl-dev openssl

WORKDIR /build

COPY ./Cargo.toml ./Cargo.lock /build/
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo fetch
RUN cargo build --release
RUN rm src/main.rs

COPY ./src/ /build/src
RUN cargo build -r

FROM gcr.io/distroless/cc-debian12
COPY --from=builder /build/target/release/course-bot /
CMD ["/course-bot"]