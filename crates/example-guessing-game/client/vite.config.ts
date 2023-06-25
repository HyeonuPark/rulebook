import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

export default defineConfig({
	plugins: [
		sveltekit(),
		{
			name: 'add cross origin isolation headers',
			configureServer(server) {
				server.middlewares.use((req, res, next) => {
					res.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
					res.setHeader('Cross-Origin-Resource-Policy', 'same-site');
					res.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
					next();
				});
			}
		}
	],
	test: {
		include: ['src/**/*.{test,spec}.{js,ts}']
	},
	server: {
		host: 'localhost',
		proxy: {
			'/room': {
				target: 'http://localhost:6667',
				changeOrigin: true,
				ws: true,
				secure: false
			}
		}
	}
});
