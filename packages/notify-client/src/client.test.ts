// ABOUTME: Tests for the notification client.
// ABOUTME: Uses Node's built-in test runner with a mock server.

import { describe, it, beforeEach, afterEach, mock } from "node:test";
import assert from "node:assert";
import { createServer, type Server, type IncomingMessage, type ServerResponse } from "node:http";
import { NotifyClient } from "./client.ts";

describe("NotifyClient", () => {
  let server: Server;
  let serverPort: number;
  let lastRequest: {
    method: string;
    url: string;
    headers: Record<string, string | string[] | undefined>;
    body: string;
  } | null = null;
  let responseStatus = 200;
  let responseBody = "";

  beforeEach(async () => {
    lastRequest = null;
    responseStatus = 200;
    responseBody = "";

    server = createServer((req: IncomingMessage, res: ServerResponse) => {
      let body = "";
      req.on("data", (chunk) => (body += chunk));
      req.on("end", () => {
        lastRequest = {
          method: req.method || "",
          url: req.url || "",
          headers: req.headers,
          body,
        };
        res.statusCode = responseStatus;
        res.setHeader("Content-Type", "application/json");
        res.end(responseBody);
      });
    });

    await new Promise<void>((resolve) => {
      server.listen(0, () => {
        const addr = server.address();
        if (addr && typeof addr === "object") {
          serverPort = addr.port;
        }
        resolve();
      });
    });
  });

  afterEach(async () => {
    await new Promise<void>((resolve) => server.close(() => resolve()));
  });

  it("sends notification with correct headers and body", async () => {
    const client = new NotifyClient({
      server: `http://localhost:${serverPort}`,
      token: "test-secret",
    });

    await client.send({
      source: "test-app",
      title: "Test Title",
      message: "Test message body",
    });

    assert.ok(lastRequest, "Request should have been made");
    assert.strictEqual(lastRequest.method, "POST");
    assert.strictEqual(lastRequest.url, "/notify");
    assert.strictEqual(lastRequest.headers.authorization, "Bearer test-secret");
    assert.strictEqual(lastRequest.headers["content-type"], "application/json");

    const body = JSON.parse(lastRequest.body);
    assert.strictEqual(body.source, "test-app");
    assert.strictEqual(body.title, "Test Title");
    assert.strictEqual(body.message, "Test message body");
  });

  it("uses defaultSource when source not specified", async () => {
    const client = new NotifyClient({
      server: `http://localhost:${serverPort}`,
      token: "test-secret",
      defaultSource: "default-app",
    });

    await client.send({
      title: "Test",
      message: "Test",
    } as any); // Cast to bypass TS requiring source

    const body = JSON.parse(lastRequest!.body);
    assert.strictEqual(body.source, "default-app");
  });

  it("notification source overrides defaultSource", async () => {
    const client = new NotifyClient({
      server: `http://localhost:${serverPort}`,
      token: "test-secret",
      defaultSource: "default-app",
    });

    await client.send({
      source: "specific-app",
      title: "Test",
      message: "Test",
    });

    const body = JSON.parse(lastRequest!.body);
    assert.strictEqual(body.source, "specific-app");
  });

  it("sends all notification fields", async () => {
    const client = new NotifyClient({
      server: `http://localhost:${serverPort}`,
      token: "test-secret",
    });

    await client.send({
      source: "test-app",
      title: "Test",
      message: "Test",
      status: "warning",
      iconHref: "https://example.com/icon.png",
      duration: 30,
      exclusive: true,
      actions: [
        {
          label: "View",
          url: "https://example.com",
          open: true,
        },
        {
          label: "Acknowledge",
          url: "https://api.example.com/ack",
          method: "POST",
          headers: { "X-Custom": "value" },
          body: '{"ack": true}',
        },
      ],
    });

    const body = JSON.parse(lastRequest!.body);
    assert.strictEqual(body.status, "warning");
    assert.strictEqual(body.iconHref, "https://example.com/icon.png");
    assert.strictEqual(body.duration, 30);
    assert.strictEqual(body.exclusive, true);
    assert.strictEqual(body.actions.length, 2);
    assert.strictEqual(body.actions[0].label, "View");
    assert.strictEqual(body.actions[0].open, true);
    assert.strictEqual(body.actions[1].method, "POST");
  });

  it("returns success result on 200", async () => {
    responseStatus = 200;

    const client = new NotifyClient({
      server: `http://localhost:${serverPort}`,
      token: "test-secret",
    });

    const result = await client.send({
      source: "test",
      title: "Test",
      message: "Test",
    });

    assert.strictEqual(result.ok, true);
    assert.strictEqual(result.status, 200);
  });

  it("returns error result on 401", async () => {
    responseStatus = 401;
    responseBody = JSON.stringify({ error: "unauthorized" });

    const client = new NotifyClient({
      server: `http://localhost:${serverPort}`,
      token: "wrong-secret",
    });

    const result = await client.send({
      source: "test",
      title: "Test",
      message: "Test",
    });

    assert.strictEqual(result.ok, false);
    assert.strictEqual(result.status, 401);
  });

  it("handles server URL with trailing slash", async () => {
    const client = new NotifyClient({
      server: `http://localhost:${serverPort}/`,
      token: "test-secret",
    });

    await client.send({
      source: "test",
      title: "Test",
      message: "Test",
    });

    assert.strictEqual(lastRequest!.url, "/notify");
  });

  it("throws on network error", async () => {
    const client = new NotifyClient({
      server: "http://localhost:1", // Invalid port
      token: "test-secret",
    });

    await assert.rejects(
      client.send({
        source: "test",
        title: "Test",
        message: "Test",
      }),
      /fetch failed/
    );
  });
});
