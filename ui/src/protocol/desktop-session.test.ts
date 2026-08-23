import { afterEach, describe, expect, it, vi } from "vitest";
import { consumeDesktopTokenFragment, currentBearer, renewDesktopSession } from "./desktop-session";

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() { return values.size; },
    clear: () => values.clear(),
    getItem: (key) => values.get(key) ?? null,
    key: (index) => [...values.keys()][index] ?? null,
    removeItem: (key) => { values.delete(key); },
    setItem: (key, value) => { values.set(key, String(value)); },
  };
}

const previousStorage = Object.getOwnPropertyDescriptor(globalThis, "sessionStorage");
const previousWindow = Object.getOwnPropertyDescriptor(globalThis, "window");

afterEach(() => {
  if (previousStorage) Object.defineProperty(globalThis, "sessionStorage", previousStorage);
  else delete (globalThis as { sessionStorage?: Storage }).sessionStorage;
  if (previousWindow) Object.defineProperty(globalThis, "window", previousWindow);
  else delete (globalThis as { window?: Window }).window;
});

describe("desktop session authentication", () => {
  it("stores and scrubs only the token fragment while preserving other fragment state", () => {
    const session = memoryStorage();
    Object.defineProperty(globalThis, "sessionStorage", { configurable: true, value: session });
    const replaceState = vi.fn();
    const location = {
      hash: "#airp-token=secret-token&panel=activity",
      pathname: "/desktop/",
      search: "?character_id=alice",
    } as Location;
    expect(consumeDesktopTokenFragment(location, { replaceState } as unknown as History)).toBe("secret-token");
    expect(currentBearer()).toBe("secret-token");
    expect(replaceState).toHaveBeenCalledWith(null, "", "/desktop/?character_id=alice#panel=activity");
  });

  it("deduplicates concurrent rotation and publishes the renewed bearer", async () => {
    const session = memoryStorage();
    session.setItem("airp_bearer", "old");
    Object.defineProperty(globalThis, "sessionStorage", { configurable: true, value: session });
    const dispatchEvent = vi.fn();
    Object.defineProperty(globalThis, "window", { configurable: true, value: { dispatchEvent } });
    const fetchImpl = vi.fn(async () => new Response(JSON.stringify({ token: "new", expires_in: 120 })));
    const [first, second] = await Promise.all([
      renewDesktopSession("http://engine.test", fetchImpl),
      renewDesktopSession("http://engine.test", fetchImpl),
    ]);
    expect([first, second]).toEqual([true, true]);
    expect(fetchImpl).toHaveBeenCalledTimes(1);
    expect(currentBearer()).toBe("new");
    expect(dispatchEvent).toHaveBeenCalledTimes(1);
  });
});

