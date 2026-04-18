import type { NextConfig } from "next";

const config: NextConfig = {
  reactStrictMode: true,
  experimental: {
    // Keep the bundle tight — we don't need server-side rendering
    // middleware for an SPA that hits one API route.
  },
};

export default config;
