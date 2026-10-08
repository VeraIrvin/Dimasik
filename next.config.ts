import type { NextConfig } from 'next';

const DEFAULT_BACKEND_URL = 'http://127.0.0.1:8787';

// The Rust backend owns all mutable data; browser `/api/*` calls keep their
// existing paths and are proxied to it, cookies included.
const backendUrl = (process.env.BACKEND_URL?.trim() || DEFAULT_BACKEND_URL).replace(/\/+$/u, '');

const nextConfig: NextConfig = {
  turbopack: { root: process.cwd() },
  devIndicators: false,
  async rewrites() {
    return [{ source: '/api/:path*', destination: `${backendUrl}/api/:path*` }];
  },
};

export default nextConfig;
