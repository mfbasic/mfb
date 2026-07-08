#!/bin/sh
# Map environment variables to `mfb-repo` CLI arguments so the server can be
# configured entirely through Fly/Docker env + secrets.
#
#   MFB_REPO_DBPATH     metadata SQLite path        (default /data/meta.db)
#   MFB_REPO_LISTEN     bind address                (default 0.0.0.0:8080)
#   MFB_REPO_DATAPATH   blob store: a local dir or s3://<bucket>/<prefix>
#   MFB_REPO_S3_PREFIX  key prefix when derived from BUCKET_NAME (default packages)
#   MFB_REPO_S3_ENDPOINT  S3 endpoint override (S3-compatible stores)
#
# Fly.io object storage (Tigris) provisioning sets BUCKET_NAME,
# AWS_ENDPOINT_URL_S3, and the AWS_* credentials automatically; when
# MFB_REPO_DATAPATH is unset we derive an s3:// data path from them, so a
# `fly storage create` + `fly deploy` needs no further blob configuration.
set -e

DBPATH="${MFB_REPO_DBPATH:-/data/meta.db}"
LISTEN="${MFB_REPO_LISTEN:-0.0.0.0:8080}"

DATAPATH="${MFB_REPO_DATAPATH:-}"
if [ -z "$DATAPATH" ] && [ -n "$BUCKET_NAME" ]; then
    DATAPATH="s3://${BUCKET_NAME}/${MFB_REPO_S3_PREFIX:-packages}"
fi
if [ -z "$DATAPATH" ]; then
    echo "error: no blob store configured — set MFB_REPO_DATAPATH (a local dir or" >&2
    echo "       s3://<bucket>/<prefix>), or provision Fly object storage so that" >&2
    echo "       BUCKET_NAME is set." >&2
    exit 2
fi

# Prefer an explicit override; otherwise fall back to the SDK-standard endpoint
# env that Fly/Tigris injects.
ENDPOINT="${MFB_REPO_S3_ENDPOINT:-${AWS_ENDPOINT_URL_S3:-}}"

set -- --dbpath "$DBPATH" --datapath "$DATAPATH" --listen "$LISTEN"
case "$DATAPATH" in
    s3://*)
        if [ -n "$ENDPOINT" ]; then
            set -- "$@" --s3-endpoint "$ENDPOINT"
        fi
        ;;
esac

echo "starting mfb-repo: dbpath=$DBPATH datapath=$DATAPATH listen=$LISTEN${ENDPOINT:+ s3-endpoint=$ENDPOINT}"
exec mfb-repo "$@"
