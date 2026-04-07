import { useState, useEffect, useCallback, useRef } from "react";

/** Provider configuration — placeholder URLs for production setup */
const PROVIDER_CONFIG = {
  moonpay: {
    name: "MoonPay",
    baseUrl: "https://buy.moonpay.com",
    apiKey: "pk_test_PLACEHOLDER", // Replace with real key in production
  },
  transak: {
    name: "Transak",
    baseUrl: "https://global.transak.com",
    apiKey: "PLACEHOLDER_API_KEY", // Replace with real key in production
  },
} as const;

export type FiatProvider = keyof typeof PROVIDER_CONFIG;

interface FiatProviderWidgetProps {
  provider: FiatProvider;
  walletAddress: string;
  fiatAmount: number;
  fiatCurrency: string;
  onClose: () => void;
  onSuccess: () => void;
  onError: (message: string) => void;
}

/**
 * Provider iframe/popup wrapper for MoonPay or Transak.
 * Builds the widget URL with parameters and handles postMessage callbacks.
 */
export function FiatProviderWidget({
  provider,
  walletAddress,
  fiatAmount,
  fiatCurrency,
  onClose,
  onSuccess,
  onError,
}: FiatProviderWidgetProps) {
  const [loading, setLoading] = useState(true);
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const config = PROVIDER_CONFIG[provider];

  // Build widget URL with parameters
  const widgetUrl = buildWidgetUrl(provider, {
    walletAddress,
    fiatAmount,
    fiatCurrency,
  });

  // Listen for postMessage callbacks from the provider widget
  const handleMessage = useCallback(
    (event: MessageEvent) => {
      // MoonPay sends events from buy.moonpay.com
      // Transak sends events from global.transak.com
      if (
        !event.origin.includes("moonpay.com") &&
        !event.origin.includes("transak.com")
      ) {
        return;
      }

      const data = event.data;
      if (!data) return;

      // MoonPay event format
      if (data.type === "moonpay_event") {
        if (data.eventName === "transaction_completed") {
          onSuccess();
        } else if (data.eventName === "transaction_failed") {
          onError("Transaction failed. Please try again.");
        }
      }

      // Transak event format
      if (data.event_id) {
        if (data.event_id === "TRANSAK_ORDER_SUCCESSFUL") {
          onSuccess();
        } else if (data.event_id === "TRANSAK_ORDER_FAILED") {
          onError("Transaction failed. Please try again.");
        } else if (data.event_id === "TRANSAK_WIDGET_CLOSE") {
          onClose();
        }
      }
    },
    [onSuccess, onError, onClose]
  );

  useEffect(() => {
    window.addEventListener("message", handleMessage);
    return () => window.removeEventListener("message", handleMessage);
  }, [handleMessage]);

  return (
    <div className="flex flex-col h-full bg-white">
      {/* Widget header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-zinc-200 bg-zinc-50">
        <span className="text-xs font-medium text-zinc-700">
          {config.name}
        </span>
        <button
          onClick={onClose}
          className="w-6 h-6 flex items-center justify-center rounded-full bg-zinc-200 hover:bg-zinc-300 transition text-xs text-zinc-600"
          title="Close"
        >
          X
        </button>
      </div>

      {/* Loading overlay */}
      {loading && (
        <div className="flex flex-col items-center justify-center flex-1 gap-3">
          <div className="w-8 h-8 rounded-full border-2 border-emerald-500 border-t-transparent animate-spin" />
          <p className="text-xs text-zinc-500">
            Loading {config.name} widget...
          </p>
        </div>
      )}

      {/* Provider iframe */}
      <iframe
        ref={iframeRef}
        src={widgetUrl}
        title={`${config.name} Widget`}
        className={`flex-1 w-full border-none ${loading ? "hidden" : ""}`}
        onLoad={() => setLoading(false)}
        onError={() => {
          setLoading(false);
          onError(
            `Failed to load ${config.name}. Please check your connection and try again.`
          );
        }}
        allow="camera;microphone;payment"
        sandbox="allow-scripts allow-same-origin allow-popups allow-forms allow-top-navigation"
      />
    </div>
  );
}

/** Build the provider-specific widget URL with query parameters */
function buildWidgetUrl(
  provider: FiatProvider,
  params: {
    walletAddress: string;
    fiatAmount: number;
    fiatCurrency: string;
  }
): string {
  const config = PROVIDER_CONFIG[provider];
  const { walletAddress, fiatAmount, fiatCurrency } = params;

  if (provider === "moonpay") {
    const query = new URLSearchParams({
      apiKey: config.apiKey,
      currencyCode: "evap",
      walletAddress,
      baseCurrencyAmount: String(fiatAmount),
      baseCurrencyCode: fiatCurrency.toLowerCase(),
      colorCode: "%2316a34a", // green-600
      theme: "light",
    });
    return `${config.baseUrl}?${query.toString()}`;
  }

  // Transak
  const query = new URLSearchParams({
    apiKey: config.apiKey,
    cryptoCurrencyCode: "EVAP",
    walletAddress,
    fiatAmount: String(fiatAmount),
    fiatCurrency: fiatCurrency.toUpperCase(),
    network: "evaporchain",
    themeColor: "16a34a",
    hideMenu: "true",
  });
  return `${config.baseUrl}?${query.toString()}`;
}
