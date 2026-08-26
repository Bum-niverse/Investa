import http from "node:http";
import { loadConfig } from "./config.mjs";
import { FirestoreRepository } from "./firestore.mjs";
import { TelegramClient } from "./telegram.mjs";
import { createRelayApp } from "./app.mjs";

const config = loadConfig();
const repository = new FirestoreRepository({ projectId: config.projectId, database: config.firestoreDatabase });
const telegram = new TelegramClient({ botToken: config.telegramBotToken });
const app = createRelayApp({ config, repository, telegram });

const server = http.createServer((request, response) => {
  const chunks = [];
  let receivedBytes = 0;
  request.on("data", (chunk) => {
    receivedBytes += chunk.length;
    if (receivedBytes <= config.maxBodyBytes) chunks.push(chunk);
  });
  request.on("end", async () => {
    try {
      const result = await app({
        method: request.method ?? "GET",
        pathname: new URL(request.url ?? "/", "http://localhost").pathname,
        headers: request.headers,
        body: Buffer.concat(chunks).toString("utf8"),
        tooLarge: receivedBytes > config.maxBodyBytes,
        remoteAddress: request.socket.remoteAddress,
      });
      response.writeHead(result.status, result.headers);
      response.end(result.body);
    } catch {
      response.writeHead(500, { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" });
      response.end(JSON.stringify({ error: "internal_error" }));
    }
  });
});

server.listen(config.port, "0.0.0.0", () => {
  console.log(`Investa cloud relay listening on ${config.port}; live orders remain disabled.`);
});
