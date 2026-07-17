# Deploying the sync server on Plesk with Docker

The server is one small binary plus a Postgres database. It listens on **two**
ports:

- **8080 — the sync API.** This is what your Booklet clients talk to; it needs to
  reach the world, behind HTTPS.
- **8081 — the admin panel.** An operations surface (users, devices, bytes, plans,
  a log). It must **not** be public by default — reach it over an SSH tunnel.

The `docker-compose.yml` binds both host ports to `127.0.0.1`, so Docker never
exposes them on a public interface. Plesk's nginx reverse-proxies the sync port
to your domain; the admin port stays on loopback.

## 1. Build and run

Over SSH on the Plesk box (Docker + the Docker Compose plugin installed — Plesk's
**Docker** extension provides them, or `apt install docker-compose-plugin`):

```sh
git clone <your repo> booklet && cd booklet
cp .env.example .env
# edit .env: set POSTGRES_PASSWORD and BOOKLET_PUBLIC_URL
docker compose up -d --build
docker compose logs -f server        # should print "sync on 0.0.0.0:8080, admin on 0.0.0.0:8081"
```

The image builds only the server crate (the QtQuick app is not built). Postgres
data and the content-addressed blob store live in named volumes (`db-data`,
`blob-data`) and survive a container replace.

## 2. Create the first admin (from the shell — nobody can sign in to make one)

```sh
docker compose exec server booklet-sync-server user create alice     # prompts for a password
docker compose exec server booklet-sync-server admin grant alice
```

## 3. Reverse-proxy the sync API (public, HTTPS)

In Plesk, create a domain or subdomain for sync, e.g. **`sync.yourdomain.com`**,
and issue a **Let's Encrypt** certificate for it (SSL/TLS Certificates). Then:

**Plesk → Domains → sync.yourdomain.com → Apache & nginx Settings →
Additional nginx directives:**

```nginx
location / {
    proxy_pass http://127.0.0.1:8080;
    proxy_http_version 1.1;
    proxy_set_header Host              $host;
    proxy_set_header X-Real-IP         $remote_addr;
    proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    client_max_body_size 100m;          # notes/blobs upload through here
}
```

Point your Booklet clients' server URL at `https://sync.yourdomain.com`. TLS
terminates here at nginx; the server behind it speaks plain HTTP on loopback,
exactly as designed.

> If Plesk reports a *duplicate `location /`*, the domain is also serving static
> files. Fix: **Apache & nginx Settings → uncheck "Serve static files directly by
> nginx"**, or use the Plesk **Docker** extension's *Proxy Rules* instead (map the
> domain to the server container's port 8080 — the extension writes the nginx
> location for you).

## 4. Reach the admin panel

### Recommended: SSH tunnel (nothing public)

From your laptop:

```sh
ssh -L 8081:127.0.0.1:8081 you@yourserver
```

Then open **http://127.0.0.1:8081/admin** and sign in. Leave `BOOKLET_PUBLIC_URL`
as `http://127.0.0.1:8081` for this mode. A device token can't open `/admin` and
the admin cookie can't reach the sync API — the two credentials are separate.

### Optional: reverse-proxy admin on its own subdomain

Only if you accept the trade (an admin login exposed to the internet). Requirements:

1. A dedicated subdomain, e.g. **`admin.yourdomain.com`**, with **Let's Encrypt SSL**
   — the session cookie is `Secure`, so it is only sent over HTTPS.
2. Set **`BOOKLET_PUBLIC_URL=https://admin.yourdomain.com`** in `.env` and
   `docker compose up -d` again (so links/return-URLs are correct).
3. Lock it down. **Apache & nginx Settings → Additional nginx directives:**

```nginx
location / {
    allow 203.0.113.7;      # your fixed IP(s)
    deny  all;              # everyone else is refused before reaching the app
    proxy_pass http://127.0.0.1:8081;
    proxy_http_version 1.1;
    proxy_set_header Host              $host;
    proxy_set_header X-Real-IP         $remote_addr;
    proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```

The panel already rate-limits sign-in and uses CSRF-protected forms, but an IP
allowlist (or Plesk's HTTP Basic auth in front) is strongly advised.

## 5. Billing webhook (only if you enable Stripe)

`POST /billing/webhook` lives on the **admin** listener (8081). If admin is
loopback-only but you use billing, expose just that one path on the public sync
domain — add to **sync.yourdomain.com**'s nginx directives:

```nginx
location = /billing/webhook {
    proxy_pass http://127.0.0.1:8081;
    proxy_set_header Host $host;
}
```

Then point the Stripe webhook at `https://sync.yourdomain.com/billing/webhook`.
It is authenticated by the Stripe signature, so exposing only this path is safe.

## 6. Updating

```sh
git pull
docker compose up -d --build       # migrations run automatically on start
```
