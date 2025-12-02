# ABOUTME: Dockerfile for the notification server.
# ABOUTME: Builds a minimal image for headless server deployment.

FROM golang:1.25-alpine AS builder

WORKDIR /build

# Copy go mod files first for layer caching
COPY go.mod go.sum ./
RUN go mod download

# Copy source
COPY . .

# Build the server binary (no CGO needed for server-only)
RUN CGO_ENABLED=0 GOOS=linux go build -o server ./cmd/server

# Runtime image
FROM alpine:3.19

RUN apk --no-cache add ca-certificates

WORKDIR /app
COPY --from=builder /build/server .

EXPOSE 9876

ENV CROSS_NOTIFIER_PORT=9876

ENTRYPOINT ["/app/server"]
