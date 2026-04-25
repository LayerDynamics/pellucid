import { GlobalRegistrator } from "@happy-dom/global-registrator";

if (typeof globalThis.document === "undefined") {
  GlobalRegistrator.register({
    url: "http://localhost/",
    width: 1280,
    height: 720,
  });
}
