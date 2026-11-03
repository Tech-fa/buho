#!/bin/bash
set -euo pipefail

: "${APP_HOST:?}"
: "${API_HOST:?}"
: "${PUBLIC_IP:?}"
: "${ADMIN_EMAIL:?}"

COMPOSE_DIR=/opt/fourss/desktop-shell/deploy
COMPOSE_FILE="$COMPOSE_DIR/docker-compose.yml"
ENV_FILE="$COMPOSE_DIR/.env"
CONFIG_FILE="$COMPOSE_DIR/ui-config.json"
SITE=/etc/nginx/sites-available/fourss

export DEBIAN_FRONTEND=noninteractive

echo "https-setup"

apt-get update
apt-get install -y python3

rewrite_ports() {
  python3 - "$COMPOSE_FILE" "$1" "$2" <<'PY'
import pathlib, sys
path, api_bind, ui_bind = sys.argv[1:]
text = pathlib.Path(path).read_text()
for old in ('"3012:3012"', '"127.0.0.1:3012:3012"'):
    text = text.replace(old, f'"{api_bind}"')
for old in ('"80:80"', '"127.0.0.1:8080:80"'):
    text = text.replace(old, f'"{ui_bind}"')
pathlib.Path(path).write_text(text)
PY
}

write_urls() {
  python3 - "$CONFIG_FILE" "$ENV_FILE" "$1" "$2" <<'PY'
import json, pathlib, sys
config_path, env_path, api_url, frontend_url = sys.argv[1:]
pathlib.Path(config_path).write_text(json.dumps({"apiUrl": api_url}) + "\n")
env = pathlib.Path(env_path)
lines = env.read_text().splitlines()
found_front = False
found_api = False
out = []
for line in lines:
    if line.startswith("FRONTEND_URL="):
        out.append("FRONTEND_URL=" + frontend_url)
        found_front = True
    elif line.startswith("API_PUBLIC_URL="):
        out.append("API_PUBLIC_URL=" + api_url)
        found_api = True
    else:
        out.append(line)
if not found_front:
    out.append("FRONTEND_URL=" + frontend_url)
if not found_api:
    out.append("API_PUBLIC_URL=" + api_url)
env.write_text("\n".join(out) + "\n")
PY
}

compose() {
  docker compose --env-file "$ENV_FILE" -f "$COMPOSE_FILE" "$@"
}

restore_http() {
  rewrite_ports "3012:3012" "80:80" || true
  write_urls "http://${PUBLIC_IP}:3012" "http://${PUBLIC_IP}" || true
  compose up -d --force-recreate api ui || true
  systemctl stop nginx || true
}

on_error() {
  trap - ERR
  restore_http
  exit 1
}

trap on_error ERR

rewrite_ports "127.0.0.1:3012:3012" "127.0.0.1:8080:80"
write_urls "https://${API_HOST}" "https://${APP_HOST}"
compose up -d --force-recreate api ui

apt-get install -y nginx certbot python3-certbot-nginx
systemctl stop nginx || true

mkdir -p /etc/nginx/conf.d
printf '%s\n' "client_max_body_size 1000m;" > /etc/nginx/conf.d/fourss-uploads.conf

# This answer is only for the certificate request. Port 80 cannot redirect until 443 has a certificate.
python3 - "$SITE" "$APP_HOST" "$API_HOST" <<'PY'
import pathlib, sys
path, app_host, api_host = sys.argv[1:]
text = f"""server {{
  listen 80 default_server;
  server_name _;
  return 444;
}}

server {{
  listen 80;
  server_name {app_host};
  location / {{
    proxy_pass http://127.0.0.1:8080;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
  }}
}}

server {{
  listen 80;
  server_name {api_host};
  client_max_body_size 1000m;
  location / {{
    proxy_pass http://127.0.0.1:3012;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_read_timeout 300s;
  }}
}}
"""
pathlib.Path(path).write_text(text)
PY

ln -sfn "$SITE" /etc/nginx/sites-enabled/fourss
rm -f /etc/nginx/sites-enabled/default
nginx -t
systemctl enable nginx
systemctl restart nginx

if command -v ufw >/dev/null 2>&1 && ufw status | grep -q "Status: active"; then
  ufw allow 80/tcp
  ufw allow 443/tcp
fi

if certbot certificates 2>/dev/null | grep -F -q "Certificate Name: ${APP_HOST}"; then
  certbot --nginx --non-interactive --redirect --reinstall --cert-name "$APP_HOST"
else
  certbot --nginx --non-interactive --agree-tos --email "$ADMIN_EMAIL" --redirect \
    -d "$APP_HOST" -d "$API_HOST"
fi

python3 - "$SITE" "$APP_HOST" "$API_HOST" <<'PY'
import pathlib, sys
path, app_host, api_host = sys.argv[1:]
certificate = f"/etc/letsencrypt/live/{app_host}/fullchain.pem"
key = f"/etc/letsencrypt/live/{app_host}/privkey.pem"
text = f"""server {{
  listen 80 default_server;
  server_name _;
  return 444;
}}

server {{
  listen 80;
  server_name {app_host};
  return 301 https://$host$request_uri;
}}

server {{
  listen 443 ssl;
  server_name {app_host};
  ssl_certificate {certificate};
  ssl_certificate_key {key};
  ssl_protocols TLSv1.2 TLSv1.3;
  location / {{
    proxy_pass http://127.0.0.1:8080;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto https;
  }}
}}

server {{
  listen 80;
  server_name {api_host};
  return 301 https://$host$request_uri;
}}

server {{
  listen 443 ssl;
  server_name {api_host};
  ssl_certificate {certificate};
  ssl_certificate_key {key};
  ssl_protocols TLSv1.2 TLSv1.3;
  client_max_body_size 1000m;
  location / {{
    proxy_pass http://127.0.0.1:3012;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto https;
    proxy_read_timeout 300s;
  }}
}}
"""
pathlib.Path(path).write_text(text)
PY

nginx -t
systemctl reload nginx

trap - ERR
echo "https-ready"
