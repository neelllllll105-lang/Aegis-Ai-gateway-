# Aegis dashboard image.
#
# Three stages: dependencies, build, runtime. Splitting dependencies from the build means
# a source-only change reuses the cached `npm ci` layer, which is most of the wall-clock
# time in a Next.js build.
#
# The runtime stage uses Next.js standalone output — a self-contained server plus only the
# `node_modules` actually reached at run time. It is roughly a tenth of the size of
# copying the whole tree, and it contains no build tooling for an attacker to use.

# ---- Dependencies ------------------------------------------------------------------
FROM node:22-alpine AS deps
WORKDIR /app

COPY package.json package-lock.json* ./
# `npm ci` rather than `npm install`: it installs exactly the lockfile, fails if the
# lockfile and manifest disagree, and never silently updates a dependency during a build.
RUN npm ci

# ---- Build -------------------------------------------------------------------------
FROM node:22-alpine AS builder
WORKDIR /app

COPY --from=deps /app/node_modules ./node_modules
COPY . .

# Next.js inlines NEXT_PUBLIC_* into the client bundle at build time, so these must be
# build arguments. Supplying them at run time has no effect, which is a confusing enough
# failure that it is worth stating here.
ARG NEXT_PUBLIC_AEGIS_API_URL
ARG NEXT_PUBLIC_SITE_URL
ENV NEXT_PUBLIC_AEGIS_API_URL=$NEXT_PUBLIC_AEGIS_API_URL
ENV NEXT_PUBLIC_SITE_URL=$NEXT_PUBLIC_SITE_URL
ENV NEXT_TELEMETRY_DISABLED=1

RUN npm run build

# ---- Runtime -----------------------------------------------------------------------
FROM node:22-alpine AS runtime
WORKDIR /app

ENV NODE_ENV=production
ENV NEXT_TELEMETRY_DISABLED=1
ENV PORT=3000
ENV HOSTNAME=0.0.0.0

# Run as a non-root user. The image has no package manager and no shell utilities worth
# reaching, but a process that cannot write outside its own directory is still a smaller
# blast radius than one that can.
RUN addgroup --system --gid 1001 nodejs \
    && adduser --system --uid 1001 nextjs

COPY --from=builder --chown=nextjs:nodejs /app/.next/standalone ./
COPY --from=builder --chown=nextjs:nodejs /app/.next/static ./.next/static
COPY --from=builder --chown=nextjs:nodejs /app/public ./public

USER nextjs
EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD node -e "fetch('http://127.0.0.1:3000/').then(r => process.exit(r.ok ? 0 : 1)).catch(() => process.exit(1))"

CMD ["node", "server.js"]
