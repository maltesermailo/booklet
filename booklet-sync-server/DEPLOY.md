# Deploying Booklet on Plesk with Docker

The server is one binary plus a Postgres database. It serves **everything on one
port** (default `8080`): the public marketing site (`/`), the user account portal
(`/account`), the sync API (`/auth/token`, `/vaults/…`, `/blobs/…`), the operator
admin (`/admin`), and the Stripe webhook (`/billing/webhook`). You put one
subdomain in front of it with HTTPS.

`docker-compose.yml` binds the host port to `127.0.0.1`, so Docker exposes nothing
publicly — Plesk's nginx reverse-proxies your subdomain to it.

## 1. Build and run

Over SSH on the Plesk box (Docker + the Compose plugin — Plesk's **Docker**
extension provides them, or `apt install docker-compose-plugin`):

```sh
git clone <your repo> booklet && cd booklet
cp .env.example .env
# edit .env: POSTGRES_PASSWORD, and BOOKLET_PUBLIC_URL = https://booklet.yourdomain.com
docker compose up -d --build
docker compose logs -f server        # should print "listening on 0.0.0.0:8080"
```

Postgres data and the content-addressed blob store live in named volumes and
survive a container replace. Migrations run automatically on start.

## 2. Create the first operator (from the shell)

Public sign-up creates ordinary users; an **operator** (admin) is made from the
shell, because nobody can sign in to grant the first one:

```sh
docker compose exec server booklet-sync-server user create alice   # prompts for a password
docker compose exec server booklet-sync-server admin grant alice
```

Everyone else just signs up at `https://booklet.yourdomain.com/signup`.

## 3. Reverse-proxy the subdomain (HTTPS)

In Plesk, create the subdomain **`booklet.yourdomain.com`** and issue a
**Let's Encrypt** certificate for it (SSL/TLS Certificates — required, because the
session cookie is `Secure`). Then:

**Plesk → Domains → booklet.yourdomain.com → Apache & nginx Settings →
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

That's it — the whole app is now at `https://booklet.yourdomain.com`:

- Marketing + pricing + sign-up: `/`, `/pricing`, `/signup`
- Account portal (usage, buy a plan): `/account`
- Operator admin (login + admin account): `/admin`
- Stripe webhook: `/billing/webhook`

Point your Booklet clients' server URL at `https://booklet.yourdomain.com`. TLS
terminates at nginx; the server behind it speaks plain HTTP on loopback.

> If Plesk reports a *duplicate `location /`*, the domain is also serving static
> files: **uncheck "Serve static files directly by nginx"** in the same settings
> page, or use the Plesk **Docker** extension's *Proxy Rules* (map the domain to
> the container's port 8080 and Plesk writes the location for you).

## 4. Billing (optional, Stripe)

Set `STRIPE_SECRET_KEY` and `STRIPE_WEBHOOK_SECRET` in `.env` and
`docker compose up -d`. Create your plans on **`/admin/plans`** (each a quota, a
shown price, and a Stripe **price id**). Point the Stripe webhook at
`https://booklet.yourdomain.com/billing/webhook`. Users then subscribe themselves
from `/pricing` or `/account` — they pay on Stripe's hosted checkout; we never
touch card data. Verifying end to end needs a real Stripe test key and webhook
forwarding (e.g. the Stripe CLI).

## 5. Email (optional, SMTP)

Set `BOOKLET_SMTP_HOST` (and `_PORT`/`_USER`/`_PASSWORD`/`_TLS`) plus
`BOOKLET_MAIL_FROM` to enable password-reset links (`/forgot`) and operator
invites. Without it, an operator sets passwords directly from `/admin`.

## 6. Updating

```sh
git pull
docker compose up -d --build       # migrations run automatically on start
```
