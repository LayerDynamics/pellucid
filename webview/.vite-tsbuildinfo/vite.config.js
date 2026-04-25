import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
const target = process.env.VITE_TARGET ?? "web";
const variant = process.env.VITE_VARIANT ?? "base";
export default defineConfig({
    plugins: [react()],
    define: {
        __VITE_TARGET__: JSON.stringify(target),
        __VITE_VARIANT__: JSON.stringify(variant),
    },
    server: {
        port: 5173,
        strictPort: true,
        host: "127.0.0.1",
        fs: {
            strict: true,
        },
        proxy: {
            "/api": {
                target: target === "desktop"
                    ? `http://127.0.0.1:${process.env.PELLUCID_SIDECAR_PORT ?? "46123"}`
                    : "http://127.0.0.1:8080",
                changeOrigin: false,
                secure: false,
            },
        },
    },
    build: {
        target: "es2022",
        sourcemap: true,
        outDir: "dist",
        emptyOutDir: true,
        chunkSizeWarningLimit: 1024,
        rollupOptions: {
            output: {
                manualChunks: {
                    react: ["react", "react-dom"],
                },
            },
        },
    },
    esbuild: {
        legalComments: "none",
    },
});
