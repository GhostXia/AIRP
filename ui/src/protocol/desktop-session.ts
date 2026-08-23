const BEARER_KEY = "airp_bearer";
const TOKEN_FRAGMENT_KEY = "airp-token";

function storage(): Storage | null {
  try {
    return typeof sessionStorage === "undefined" ? null : sessionStorage;
  } catch {
    return null;
  }
}

/** Consume the shell-delivered bearer without ever putting it in a request URL. */
export function consumeDesktopTokenFragment(
  location: Location = window.location,
  historyImpl: History = window.history,
): string | null {
  const fragment = new URLSearchParams(location.hash.replace(/^#/, ""));
  const token = fragment.get(TOKEN_FRAGMENT_KEY);
  if (!token) return null;
  storage()?.setItem(BEARER_KEY, token);
  fragment.delete(TOKEN_FRAGMENT_KEY);
  const suffix = fragment.toString();
  historyImpl.replaceState(null, "", `${location.pathname}${location.search}${suffix ? `#${suffix}` : ""}`);
  return token;
}

export function currentBearer(): string {
  return storage()?.getItem(BEARER_KEY) ?? "";
}

let renewal: Promise<boolean> | null = null;

/** Rotate an expired desktop-session token. Concurrent 401s share one request. */
export function renewDesktopSession(
  base = location.origin,
  fetchImpl: typeof fetch = fetch,
): Promise<boolean> {
  if (renewal !== null) return renewal;
  const oldToken = currentBearer();
  if (!oldToken) return Promise.resolve(false);
  renewal = fetchImpl(new URL("/v1/desktop-session/renew", base), {
    method: "POST",
    headers: { Authorization: `Bearer ${oldToken}` },
    signal: AbortSignal.timeout(10_000),
  }).then(async (response) => {
    if (!response.ok) return false;
    const body = await response.json() as { token?: unknown; expires_in?: unknown };
    if (typeof body.token !== "string" || body.token.length === 0) return false;
    storage()?.setItem(BEARER_KEY, body.token);
    window.dispatchEvent(new CustomEvent("airp-bearer-renewed", {
      detail: { expires_in: body.expires_in },
    }));
    return true;
  }).catch(() => false).finally(() => {
    renewal = null;
  });
  return renewal;
}
