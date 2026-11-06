#!/bin/sh
# A built image started as docker-compose.yml runs it, on a data folder root made:
# healthy, as its own user (uid 1000) with none of root's powers left, answering its
# page and /api/status with the version given; then who it answers, on a local
# network and from the internet (bin/container-network.test.sh). A release's run
# checks the image it pushed so before publishing the release (release.yml,
# image-check), and CI checks the image it built so on every pull request
# (tests.yml, container). Run from the repository's root, where Docker and sudo are:
#
#   sh bin/image-check.sh <image> <version>
set -eu
IMAGE=$1
VERSION=$2
PORT=8793
work=$(mktemp -d)
files="-p bagholder-check -f docker-compose.yml -f $work/check.yml"
done_() {
  status=$?
  [ "$status" -ne 0 ] && docker logs bagholder-check 2>&1 | tail -n 50
  BAGHOLDER_PORT=$PORT docker compose $files down > /dev/null 2>&1 || true
  sudo rm -rf "$work"
  exit "$status"
}
trap done_ EXIT
sudo mkdir -m 700 "$work/data"
cat > "$work/check.yml" <<EOF
services:
  bagholder:
    image: $IMAGE
    container_name: bagholder-check
    environment:
      - BAGHOLDER_OFFLINE=1
      - BAGHOLDER_DRY_ORDERS=1
    volumes: !override
      - $work/data:/data
EOF
BAGHOLDER_PORT=$PORT docker compose $files up -d --wait --wait-timeout 300
docker exec bagholder-check grep -qE '^Uid:\s+1000\s+1000\s+1000\s+1000$' /proc/1/status || { echo "the app does not run as uid 1000"; exit 1; }
docker exec bagholder-check grep -qE '^CapEff:\s+0+$' /proc/1/status || { echo "the app kept some of root's powers"; exit 1; }
curl -sf -o /dev/null "http://127.0.0.1:$PORT/" || { echo "the page is not served"; exit 1; }
said=$(curl -sf -H 'X-Bagholder: 1' "http://127.0.0.1:$PORT/api/status")
echo "$said" | jq -e --arg v "$VERSION" '.version == $v' > /dev/null || { echo "the image says it is $(echo "$said" | jq -r .version), not $VERSION"; exit 1; }
BAGHOLDER_PORT=$PORT docker compose $files down
sh bin/container-network.test.sh "$IMAGE"
echo "$IMAGE: healthy as uid 1000, its page and version $VERSION served, and answering whom it should"
