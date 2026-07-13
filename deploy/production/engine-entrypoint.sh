#!/bin/sh
set -eu

read_secret() {
  secret_path="$1"
  variable_name="$2"
  required="$3"
  if [ ! -r "$secret_path" ]; then
    if [ "$required" = "required" ]; then
      echo "missing required secret: $secret_path" >&2
      exit 1
    fi
    return
  fi
  secret_value=$(cat "$secret_path")
  if [ -z "$secret_value" ]; then
    if [ "$required" = "required" ]; then
      echo "required secret is empty: $secret_path" >&2
      exit 1
    fi
    return
  fi
  export "$variable_name=$secret_value"
}

read_secret /run/secrets/engine_access_key AIRP_ACCESS_KEY required
read_secret /run/secrets/provider_api_key AIRP_API_KEY optional
exec /usr/local/bin/airp-core "$@"
