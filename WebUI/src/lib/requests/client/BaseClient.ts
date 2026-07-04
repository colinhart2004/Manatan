/*
 * Copyright (C) Contributors to the Suwayomi project
 *
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

import { AppStorage } from '@/lib/storage/AppStorage.ts';
import { UserRefreshMutation } from '@/lib/requests/types.ts';
import { AuthManager } from '@/features/authentication/AuthManager.ts';
import { AbortableApolloMutationResponse } from '@/lib/requests/RequestManager.ts';
import { SubpathUtil } from '@/lib/utils/SubpathUtil.ts';
import { ControlledPromise } from '@/lib/ControlledPromise.ts';

interface QueuedRequest {
    execute: () => void;
    resolve: (value: any) => void;
    reject: (error: any) => void;
}

export abstract class BaseClient<Client, ClientConfig, Fetcher> {
    static readonly BASE_URL_KEY = 'serverBaseURL';

    protected abstract client: Client;

    public abstract readonly fetcher: Fetcher;

    private static activeTokenRefreshPromise: Promise<UserRefreshMutation | null | undefined> | null = null;

    private static onTokenRefreshComplete: (() => void) | null = null;

    protected requestQueue: QueuedRequest[] = [];

    public reset(): void {
        BaseClient.activeTokenRefreshPromise = null;
        this.clearQueue(new Error('Client reset'));
    }

    public static setTokenRefreshCompleteCallback(callback: (() => void) | null): void {
        BaseClient.onTokenRefreshComplete = callback;
    }

    protected static async refreshAccessToken(
        refreshFn: (refreshToken: string) => AbortableApolloMutationResponse<UserRefreshMutation>,
    ): Promise<UserRefreshMutation | null | undefined> {
        const refreshToken = AuthManager.getRefreshToken();

        if (!AuthManager.isAuthInitialized()) {
            AuthManager.setAuthInitialized(true);
            AuthManager.setAuthRequired(true);
        }

        if (!refreshToken) {
            throw new Error('No refresh token found');
        }

        if (this.activeTokenRefreshPromise) {
            return this.activeTokenRefreshPromise;
        }

        AuthManager.setIsRefreshingToken(true);

        const refreshRequest = refreshFn(refreshToken).response;
        this.activeTokenRefreshPromise = refreshRequest.then((result) => result.data);

        try {
            const result = await refreshRequest;
            const { data } = result;

            if (!data) {
                throw new Error('No refreshed access token returned');
            }

            AuthManager.setAccessToken(data.refreshToken.accessToken);

            BaseClient.onTokenRefreshComplete?.();

            return data;
        } catch (e) {
            AuthManager.removeTokens();
            throw e;
        } finally {
            this.activeTokenRefreshPromise = null;
            AuthManager.setIsRefreshingToken(false);
        }
    }

    protected constructor(
        protected handleRefreshToken: (refreshToken: string) => AbortableApolloMutationResponse<UserRefreshMutation>,
    ) {}

    public static getDefaultBaseUrl(): string {
        if (import.meta.env.DEV) {
            return import.meta.env.VITE_SERVER_URL_DEFAULT;
        }

        return window.location.origin;
    }

    private static getEffectivePort(url: URL): string {
        if (url.port) {
            return url.port;
        }

        if (url.protocol === 'http:') {
            return '80';
        }

        if (url.protocol === 'https:') {
            return '443';
        }

        return '';
    }

    private static isLoopbackHost(hostname: string): boolean {
        const normalizedHostname = hostname.toLowerCase().replace(/^\[(.*)]$/, '$1');

        return (
            normalizedHostname === 'localhost' ||
            normalizedHostname === '127.0.0.1' ||
            normalizedHostname === '::1' ||
            normalizedHostname === '0:0:0:0:0:0:0:1'
        );
    }

    private static shouldUseCurrentOriginForStoredBaseUrl(storedUrl: URL, currentUrl: URL): boolean {
        if (import.meta.env.DEV) {
            return false;
        }

        if (BaseClient.getEffectivePort(storedUrl) !== BaseClient.getEffectivePort(currentUrl)) {
            return false;
        }

        const sameHostname = storedUrl.hostname.toLowerCase() === currentUrl.hostname.toLowerCase();
        const equivalentLoopbackHost =
            BaseClient.isLoopbackHost(storedUrl.hostname) && BaseClient.isLoopbackHost(currentUrl.hostname);

        if (!sameHostname && !equivalentLoopbackHost) {
            return false;
        }

        return storedUrl.protocol !== currentUrl.protocol || storedUrl.host !== currentUrl.host;
    }

    private static getMigratedStoredBaseUrl(serverBaseURL: string, defaultUrl: string): string {
        if (import.meta.env.DEV) {
            return serverBaseURL;
        }

        try {
            const storedUrl = new URL(serverBaseURL);
            const currentUrl = new URL(window.location.origin);

            if (!BaseClient.shouldUseCurrentOriginForStoredBaseUrl(storedUrl, currentUrl)) {
                return serverBaseURL;
            }

            const storedPath = storedUrl.pathname === '/' ? '' : storedUrl.pathname;
            return `${currentUrl.origin}${storedPath}${storedUrl.search}${storedUrl.hash}`;
        } catch {
            return defaultUrl;
        }
    }

    public getBaseUrl(): string {
        const defaultUrl = BaseClient.getDefaultBaseUrl();
        const storedBaseURL = AppStorage.local.getItemParsed(BaseClient.BASE_URL_KEY, defaultUrl);
        const serverBaseURL = BaseClient.getMigratedStoredBaseUrl(storedBaseURL, defaultUrl);

        if (serverBaseURL !== storedBaseURL) {
            AppStorage.local.setItem(BaseClient.BASE_URL_KEY, serverBaseURL, false);
        }

        // Apply subpath configuration to the base URL
        return SubpathUtil.getApiBaseUrl(serverBaseURL);
    }

    // eslint-disable-next-line @typescript-eslint/no-unused-vars
    protected shouldQueueRequest(operationName?: string): boolean {
        if (operationName?.includes('/api/v1/about')) {
            return false;
        }
        const shouldQueue = AuthManager.shouldQueueRequests();
        if (shouldQueue) {
            console.info('[request] queueing request', {
                operation: operationName,
                authInitialized: AuthManager.isAuthInitialized(),
                authRequired: AuthManager.isAuthRequired(),
                refreshingToken: AuthManager.isRefreshingToken(),
            });
        }
        return shouldQueue;
    }

    protected enqueueRequest<T>(executor: () => Promise<T>, operationName?: string): Promise<T> {
        if (!this.shouldQueueRequest(operationName)) {
            return executor();
        }

        const queuedRequest = new ControlledPromise<T>();
        const resolve = queuedRequest.resolve.bind(queuedRequest);
        const reject = queuedRequest.reject.bind(queuedRequest);

        this.requestQueue.push({
            execute: () => {
                executor().then(resolve).catch(reject);
            },
            resolve,
            reject,
        });

        console.info('[request] queued request count', { count: this.requestQueue.length });

        return queuedRequest.promise;
    }

    public processQueue(): void {
        const queue = [...this.requestQueue];
        this.requestQueue = [];

        if (queue.length) {
            console.info('[request] processing queued requests', { count: queue.length });
        }
        queue.forEach((request) => {
            request.execute();
        });
    }

    protected clearQueue(error?: Error): void {
        const queue = [...this.requestQueue];
        this.requestQueue = [];

        queue.forEach((request) => {
            request.reject(error ?? new Error('Request queue cleared'));
        });
    }

    public abstract updateConfig(config: Partial<ClientConfig>): void;
}
