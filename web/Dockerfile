# ── Build stage ───────────────────────────────────────────────────────────────
FROM node:22-slim AS builder

WORKDIR /build
COPY package.json package-lock.json ./
RUN npm ci

COPY . .
RUN npm run build

# ── Runtime stage ─────────────────────────────────────────────────────────────
FROM node:22-slim

WORKDIR /app
COPY --from=builder /build/build ./build
COPY --from=builder /build/package.json ./
COPY --from=builder /build/node_modules ./node_modules

EXPOSE 3000
ENV NODE_ENV=production

CMD ["node", "build"]
