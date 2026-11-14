import { beforeEach, expect, test, vi } from "vitest";

const push = vi.fn();
vi.mock("vue-router", () => ({ useRouter: () => ({ push }) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("@tauri-apps/plugin-deep-link", () => ({ onOpenUrl: vi.fn(), getCurrent: vi.fn() }));
vi.mock("@/stores/setting", () => ({ useSettingStore: () => ({}) }));

const { useExternalImports } = await import("@/composables/useExternalImports");

beforeEach(() => push.mockClear());

const lastQuery = () => push.mock.lastCall?.[0]?.query as Record<string, string>;

test("deep links only fill in the URL and ask for confirmation", () => {
  useExternalImports().handleDeepLink("ytdlp-gui://download?url=https%3A%2F%2Fexample.com%2Fvideo");
  expect(lastQuery()).toMatchObject({ url: "https://example.com/video", source: "deeplink" });
});

test("deep links forwarded as launch arguments also ask for confirmation", () => {
  const { handleCliOpenRequest } = useExternalImports();
  handleCliOpenRequest({
    url: "https://example.com/video",
    cookieFile: null,
    downloadDir: null,
    fromDeepLink: true,
  });
  expect(lastQuery()).toMatchObject({ source: "deeplink" });

  handleCliOpenRequest({ url: "https://example.com/video", cookieFile: null, downloadDir: null });
  expect(lastQuery()).not.toHaveProperty("source");
});

test("browser extension imports keep starting immediately", () => {
  useExternalImports().handleBrowserExtensionImport({
    url: "https://example.com/video",
    urls: ["https://example.com/video"],
    requestId: "request-1",
    cookieCount: 0,
  } as never);
  expect(lastQuery()).not.toHaveProperty("source");
});
