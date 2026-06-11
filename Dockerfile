FROM alpine:3.20

ARG BINARY_PATH=target/x86_64-unknown-linux-musl/release/markdown-preview-service

RUN apk add --no-cache fontconfig ca-certificates \
    && addgroup -S app \
    && adduser -S app -G app

WORKDIR /app

COPY ${BINARY_PATH} /usr/local/bin/markdown-preview-service

RUN chmod +x /usr/local/bin/markdown-preview-service

USER app

ENV PORT=3001
EXPOSE 3001

CMD ["/usr/local/bin/markdown-preview-service"]
