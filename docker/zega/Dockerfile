# syntax=docker/dockerfile:1
# Zega as a plain server: the image promote.yml publishes as
# addiscorp/zega:<version>. It refuses to start without a token (ZEGA_TOKEN,
# or a file mounted at /etc/zega/token) and serves on 0.0.0.0:9342. pg_zega
# and Simien's local stack run it as an ordinary Zega server.
FROM rust:1.96.0-bookworm AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/.target \
    cargo build --locked --release -p zega-cli --target-dir /src/.target \
 && install -m 0755 /src/.target/release/zega-server /zega-server

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 # Debian #1151413: require the eMudhra/AAA distrust update (or newer).
 && dpkg --compare-versions "$(dpkg-query -W -f='${Version}' ca-certificates)" ge \
      '20250419~deb12u2' \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --no-create-home --shell /usr/sbin/nologin --uid 10001 --user-group zega \
 && mkdir -p /var/lib/zega /etc/zega \
 && chown zega:zega /var/lib/zega \
 && touch /etc/zega/token \
 && chown zega:zega /etc/zega/token \
 && chmod 0600 /etc/zega/token
COPY --from=build /zega-server /usr/local/bin/zega-server
COPY docker/zega/entrypoint.sh /usr/local/bin/zega-entrypoint.sh
RUN chmod 0755 /usr/local/bin/zega-entrypoint.sh
EXPOSE 9342
ENTRYPOINT ["/usr/local/bin/zega-entrypoint.sh"]
