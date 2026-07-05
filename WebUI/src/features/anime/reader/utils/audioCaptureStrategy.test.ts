import assert from 'node:assert/strict';
import test from 'node:test';

import {
    shouldTryBrowserAudioCapture,
    shouldTryServerAudioCapture,
} from '@/features/anime/reader/utils/audioCaptureStrategy.ts';

test('direct-video browser audio capture still tries the server first on desktop', () => {
    assert.equal(
        shouldTryServerAudioCapture({
            isAndroid: false,
            isDesktopPlatform: true,
            isHlsSource: false,
            isNativeApp: false,
        }),
        true,
    );
});

test('desktop direct-video audio capture falls back to browser capture after server capture fails', () => {
    assert.equal(
        shouldTryBrowserAudioCapture({
            isAndroid: false,
            isDesktopPlatform: true,
            isHlsSource: false,
            isNativeApp: false,
            hasServerAudio: false,
        }),
        true,
    );
});

test('hls audio capture stays server-only after server capture fails', () => {
    assert.equal(
        shouldTryBrowserAudioCapture({
            isAndroid: false,
            isDesktopPlatform: true,
            isHlsSource: true,
            isNativeApp: false,
            hasServerAudio: false,
        }),
        false,
    );
});

test('plain web hls audio capture keeps browser fallback after server capture fails', () => {
    assert.equal(
        shouldTryBrowserAudioCapture({
            isAndroid: false,
            isDesktopPlatform: false,
            isHlsSource: true,
            isNativeApp: false,
            hasServerAudio: false,
        }),
        true,
    );
});

test('successful server audio capture does not fall back to browser capture', () => {
    assert.equal(
        shouldTryBrowserAudioCapture({
            isAndroid: false,
            isDesktopPlatform: true,
            isHlsSource: false,
            isNativeApp: false,
            hasServerAudio: true,
        }),
        false,
    );
});
