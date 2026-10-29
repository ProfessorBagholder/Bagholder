#!/bin/sh
# Who the image answers, on real Docker networks (docs/decisions.md, 2026-10-30):
# the local network, directly or through a reverse proxy, and never the internet.
# A local network (192.168.250.0/24) and a stand-in for the internet (203.0.113.0/24,
# a range set aside for documentation) each have a client. Bagholder and four
# proxies stand on both: Traefik, Caddy, nginx as it is usually set up for a proxy
# (the visitor's address passed on), and nginx as it comes (HTTP/1.0, no visitor
# named). Every case is a request for /api/status, answered (200) or refused (403).
# The image is the first argument (bagholder:ci in CI).
set -eu
IMAGE=${1:-bagholder:ci}
P=bhnet
gone() {
  docker rm -f $P-app $P-traefik $P-caddy $P-nginx $P-nginx-bare > /dev/null 2>&1 || true
  docker network rm $P-lan $P-net > /dev/null 2>&1 || true
}
gone
DIR=$(mktemp -d)
trap 'gone; rm -rf "$DIR"' EXIT

docker network create --subnet 192.168.250.0/24 $P-lan > /dev/null
docker network create --subnet 203.0.113.0/24 $P-net > /dev/null
both() { docker network connect --ip "$2" $P-net "$1"; }

docker run -d --name $P-app --network $P-lan --ip 192.168.250.10 \
  -e BAGHOLDER_OFFLINE=1 -e BAGHOLDER_DRY_ORDERS=1 -e BAGHOLDER_NOTIFY=browser "$IMAGE" > /dev/null
both $P-app 203.0.113.10

cat > "$DIR/traefik.yml" <<'EOF'
http:
  routers:
    bagholder:
      rule: Host(`bagholder.example.com`)
      entryPoints: [web]
      service: bagholder
  services:
    bagholder:
      loadBalancer:
        servers:
          - url: http://bhnet-app:8765
EOF
docker run -d --name $P-traefik --network $P-lan --ip 192.168.250.11 -v "$DIR/traefik.yml:/dynamic.yml:ro" \
  traefik:v3.7.16 --entryPoints.web.address=:80 --providers.file.filename=/dynamic.yml > /dev/null
both $P-traefik 203.0.113.11

printf ':80 {\n  reverse_proxy bhnet-app:8765\n}\n' > "$DIR/Caddyfile"
docker run -d --name $P-caddy --network $P-lan --ip 192.168.250.12 -v "$DIR/Caddyfile:/etc/caddy/Caddyfile:ro" caddy:2.10.2 > /dev/null
both $P-caddy 203.0.113.12

cat > "$DIR/nginx.conf" <<'EOF'
server {
  listen 80;
  location / {
    proxy_pass http://bhnet-app:8765;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
  }
}
EOF
docker run -d --name $P-nginx --network $P-lan --ip 192.168.250.13 -v "$DIR/nginx.conf:/etc/nginx/conf.d/default.conf:ro" nginx:1.29.3 > /dev/null
both $P-nginx 203.0.113.13

printf 'server {\n  listen 80;\n  location / { proxy_pass http://bhnet-app:8765; }\n}\n' > "$DIR/nginx-bare.conf"
docker run -d --name $P-nginx-bare --network $P-lan --ip 192.168.250.14 -v "$DIR/nginx-bare.conf:/etc/nginx/conf.d/default.conf:ro" nginx:1.29.3 > /dev/null
both $P-nginx-bare 203.0.113.14

# a request from a client on `network` to `to`, asking for `host`: its status code
ask() {
  network=$1 to=$2 host=$3
  shift 3
  docker run --rm --network $P-$network curlimages/curl:8.17.0 -s -o /dev/null -w '%{http_code}' --max-time 10 -H "Host: $host" "$@" "http://$to/api/status"
}
failed=0
expect() {
  want=$1 what=$2
  shift 2
  got=$(ask "$@")
  if [ "$got" = "$want" ]; then echo "ok    $want  $what"; else echo "FAIL  $what: wanted $want, got $got"; failed=1; fi
}

# the app comes up: any answer at all
i=0
until [ "$(ask lan 192.168.250.10:8765 192.168.250.10:8765)" != 000 ]; do
  i=$((i + 1))
  [ $i -lt 60 ] || { docker logs $P-app; exit 1; }
  sleep 1
done

expect 200 "directly, from the local network, at its address" lan 192.168.250.10:8765 192.168.250.10:8765
expect 200 "directly, from the local network, by a one-word name" lan bhnet-app:8765 bhnet-app:8765
expect 403 "directly, a website's own name pointed at it" lan 192.168.250.10:8765 evil.example
expect 403 "directly, from the internet" net 203.0.113.10:8765 192.168.250.10:8765
expect 403 "directly, from the internet, naming the app's own loopback" net 203.0.113.10:8765 127.0.0.1:8765
for proxy in traefik:11 caddy:12 nginx:13; do
  name=${proxy%%:*} n=${proxy##*:}
  expect 200 "through $name, from the local network, by its name" lan 192.168.250.$n bagholder.example.com
  expect 403 "through $name, from the internet" net 203.0.113.$n bagholder.example.com
  expect 403 "through $name, from the internet, claiming a local address" net 203.0.113.$n bagholder.example.com -H "X-Forwarded-For: 192.168.250.99"
done
expect 403 "through nginx as it comes, from the local network: it names no visitor" lan 192.168.250.14 bagholder.example.com
expect 403 "through nginx as it comes, from the internet" net 203.0.113.14 bagholder.example.com

[ $failed = 0 ] || { docker logs $P-app 2>&1 | tail -20; exit 1; }
echo "every case as it should be"
