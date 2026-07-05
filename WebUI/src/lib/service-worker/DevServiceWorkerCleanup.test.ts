import assert from 'node:assert/strict';
import test from 'node:test';

import {
    cleanupDevServiceWorkers,
    DEV_SERVICE_WORKER_CLEANUP_RELOAD_KEY,
} from '@/lib/service-worker/DevServiceWorkerCleanup.ts';

const setGlobal = (key: keyof typeof globalThis, value: unknown) => {
    Object.defineProperty(globalThis, key, {
        configurable: true,
        value,
    });
};

test('cleanupDevServiceWorkers unregisters dev service workers, clears dev caches, and reloads controlled pages once', async () => {
    let unregisterCalls = 0;
    let reloadCalls = 0;
    const deletedCaches: string[] = [];
    const sessionValues = new Map<string, string>();

    setGlobal('navigator', {
        serviceWorker: {
            controller: {},
            getRegistrations: async () => [
                {
                    unregister: async () => {
                        unregisterCalls += 1;
                        return true;
                    },
                },
                {
                    unregister: async () => {
                        unregisterCalls += 1;
                        return true;
                    },
                },
            ],
        },
    });
    setGlobal('caches', {
        keys: async () => ['app-shell', 'image-cache', 'workbox-precache-v2-test', 'keep-me'],
        delete: async (cacheName: string) => {
            deletedCaches.push(cacheName);
            return true;
        },
    });
    setGlobal('sessionStorage', {
        getItem: (key: string) => sessionValues.get(key) ?? null,
        setItem: (key: string, value: string) => sessionValues.set(key, value),
        removeItem: (key: string) => sessionValues.delete(key),
    });
    setGlobal('window', {
        location: {
            reload: () => {
                reloadCalls += 1;
            },
        },
    });

    await cleanupDevServiceWorkers(true);

    assert.equal(unregisterCalls, 2);
    assert.deepEqual(deletedCaches, ['app-shell', 'image-cache', 'workbox-precache-v2-test']);
    assert.equal(sessionValues.get(DEV_SERVICE_WORKER_CLEANUP_RELOAD_KEY), 'true');
    assert.equal(reloadCalls, 1);

    await cleanupDevServiceWorkers(true);

    assert.equal(reloadCalls, 1);
});

test('cleanupDevServiceWorkers does nothing outside dev mode', async () => {
    let getRegistrationsCalls = 0;

    setGlobal('navigator', {
        serviceWorker: {
            controller: {},
            getRegistrations: async () => {
                getRegistrationsCalls += 1;
                return [];
            },
        },
    });

    await cleanupDevServiceWorkers(false);

    assert.equal(getRegistrationsCalls, 0);
});
