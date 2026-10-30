FROM node:24-bookworm AS build

WORKDIR /app
COPY documents_dashboard/package.json documents_dashboard/package-lock.json ./
# The dashboard lockfile is written by npm 11. Node 22 ships npm 10, which rejects it.
RUN npm install -g npm@11.8.0 && npm ci

COPY documents_dashboard/ ./
RUN npm run build-only

FROM nginx:1.27-alpine

COPY desktop-shell/deploy/nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /app/dist /usr/share/nginx/html
COPY desktop-shell/deploy/ui-config.json /usr/share/nginx/html/config.json
