# ── Stage 1: build the React app ─────────────────────────────────────────────
# Bun is used instead of npm for faster installs. It also resolves platform-specific
# optional deps (e.g. @rollup/rollup-linux-x64-gnu) correctly on Linux regardless
# of where the lockfile was generated, avoiding the cross-platform npm lockfile bug.
FROM oven/bun:1 AS build

WORKDIR /app

# Install dependencies first (cached unless package.json / package-lock.json change)
COPY package.json bun.lock ./
RUN bun install --frozen-lockfile

# Copy source and build
COPY . .
RUN bun run build

# ── Stage 2: serve with nginx ─────────────────────────────────────────────────
FROM nginx:1.27-alpine AS runtime

# Replace the default nginx config with our own
COPY nginx.conf /etc/nginx/conf.d/default.conf

# Copy built assets from the build stage
COPY --from=build /app/dist /usr/share/nginx/html

EXPOSE 80

CMD ["nginx", "-g", "daemon off;"]
