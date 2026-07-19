// `export {}` makes this a module, which is what allows the top-level `await` below. It has
// nothing to export.
export {};

const DEFAULT_PORT = 3883;

const input = document.querySelector<HTMLInputElement>("#port");
// Not `status`: that name is `window.status`, a string, so declaring it here collides with a
// global rather than shadowing it.
const statusLine = document.querySelector<HTMLParagraphElement>("#status");
if (input === null || statusLine === null) {
  throw new Error("the options page is missing its input");
}

const { port } = await chrome.storage.local.get({ port: DEFAULT_PORT });
input.value = String(port);

input.addEventListener("change", () => {
  const chosen = Number(input.value);
  if (!Number.isInteger(chosen) || chosen < 1 || chosen > 65535) {
    statusLine.textContent = "not a port number";
    return;
  }
  void chrome.storage.local.set({ port: chosen }).then(() => {
    statusLine.textContent = `saved: mercury on ${String(chosen)}`;
  });
});
