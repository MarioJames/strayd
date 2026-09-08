FROM oven/bun:1.3.12@sha256:8956c7667fa17beb6e3c664115e66bdacfe502da5d99603626e74c197bdef160 AS bun
FROM node:22-bookworm-slim@sha256:d649c27dae7ba0137b3cef5dd75baa422c08dc3d9e3fc0c23dfb172dc3cc6436 AS node
FROM rust:1.98.0-bookworm@sha256:82150a52ec202c1b14d7817e14516c392bb7f5cfebd88f1ed531cb37ebd39922
COPY --from=bun /usr/local/bin/bun /usr/local/bin/bun
COPY --from=node /usr/local/bin/node /usr/local/bin/node
RUN apt-get update && apt-get install -y --no-install-recommends python3 python3-paramiko openjdk-17-jdk-headless openssh-client
WORKDIR /app
COPY . .
RUN cargo build --locked -p port-deck-cli -p strayd-test-support && cargo test --locked --workspace \
    && dpkg-query -W > /app/dependency-versions.txt \
    && useradd --uid 10001 --create-home tester && mkdir /evidence && chown tester /evidence
USER tester
ENTRYPOINT ["/app/target/debug/strayd-test-runner", "--repository", "/app", "--strayd", "/app/target/debug/strayd", "--fixture", "/app/target/debug/fixture-process", "--scratch-root", "/tmp", "--output", "/evidence"]
CMD ["--suite", "native"]
