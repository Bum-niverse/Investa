FROM node:22-bookworm-slim

WORKDIR /app
COPY package.json runner.mjs market-health.mjs ./

USER node
ENTRYPOINT ["node", "runner.mjs"]
