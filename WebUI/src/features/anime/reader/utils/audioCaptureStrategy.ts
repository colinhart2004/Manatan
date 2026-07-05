type AudioCaptureStrategyInput = {
    isAndroid: boolean;
    isDesktopPlatform: boolean;
    isHlsSource: boolean;
    isNativeApp: boolean;
};

type BrowserAudioCaptureInput = AudioCaptureStrategyInput & {
    hasServerAudio: boolean;
};

const isServerPreferredPlatform = ({
    isAndroid,
    isDesktopPlatform,
    isNativeApp,
}: Pick<AudioCaptureStrategyInput, 'isAndroid' | 'isDesktopPlatform' | 'isNativeApp'>) =>
    isAndroid || isDesktopPlatform || isNativeApp;

export const shouldTryServerAudioCapture = ({
    isAndroid,
    isDesktopPlatform,
    isHlsSource,
    isNativeApp,
}: AudioCaptureStrategyInput) =>
    isHlsSource || isServerPreferredPlatform({ isAndroid, isDesktopPlatform, isNativeApp });

export const shouldTryBrowserAudioCapture = ({
    hasServerAudio,
    isAndroid,
    isDesktopPlatform,
    isHlsSource,
    isNativeApp,
}: BrowserAudioCaptureInput) =>
    !hasServerAudio && (!isHlsSource || !isServerPreferredPlatform({ isAndroid, isDesktopPlatform, isNativeApp }));
