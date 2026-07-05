export const DEV_SERVICE_WORKER_CLEANUP_RELOAD_KEY = 'manatan-dev-sw-cleanup-reloaded';

const DEV_SERVICE_WORKER_CACHE_NAMES = ['app-shell', 'image-cache'];
const DEV_SERVICE_WORKER_CACHE_PREFIXES = ['workbox-precache'];

const shouldDeleteDevCache = (cacheName: string) =>
    DEV_SERVICE_WORKER_CACHE_NAMES.includes(cacheName) ||
    DEV_SERVICE_WORKER_CACHE_PREFIXES.some((prefix) => cacheName.startsWith(prefix));

export async function cleanupDevServiceWorkers(isDev: boolean): Promise<void> {
    if (!isDev || typeof navigator === 'undefined' || !('serviceWorker' in navigator)) {
        return;
    }

    try {
        const { serviceWorker } = navigator;
        const hadController = Boolean(serviceWorker.controller);
        const registrations = await serviceWorker.getRegistrations();
        const unregisterResults = await Promise.all(registrations.map((registration) => registration.unregister()));

        if (typeof caches !== 'undefined') {
            const cacheNames = await caches.keys();
            await Promise.all(cacheNames.filter(shouldDeleteDevCache).map((cacheName) => caches.delete(cacheName)));
        }

        if (typeof sessionStorage !== 'undefined' && !hadController) {
            sessionStorage.removeItem(DEV_SERVICE_WORKER_CLEANUP_RELOAD_KEY);
        }

        const didUnregisterServiceWorker = unregisterResults.some(Boolean);
        if (
            !hadController ||
            !didUnregisterServiceWorker ||
            typeof sessionStorage === 'undefined' ||
            typeof window === 'undefined'
        ) {
            return;
        }

        if (sessionStorage.getItem(DEV_SERVICE_WORKER_CLEANUP_RELOAD_KEY)) {
            return;
        }

        sessionStorage.setItem(DEV_SERVICE_WORKER_CLEANUP_RELOAD_KEY, 'true');
        window.location.reload();
    } catch (error) {
        // eslint-disable-next-line no-console
        console.warn('[service-worker] dev cleanup failed', error);
    }
}
