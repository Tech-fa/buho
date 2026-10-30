FROM node:22-bookworm

WORKDIR /app

COPY documents_backend/package.json documents_backend/package-lock.json ./
RUN npm ci

COPY documents_backend/ ./
RUN npm run build

ENV NODE_ENV=production
EXPOSE 3012

CMD ["sh", "-c", "node ./node_modules/typeorm/cli.js migration:run -d ./dist/db/database.js && node dist/main.js"]
