import { useEffect } from "react";
import { useWallet } from "@/hooks/useWallet";
import { LockScreen } from "@/components/LockScreen";
import { CreateAccount } from "@/components/CreateAccount";
import { ImportAccount } from "@/components/ImportAccount";
import { HomeScreen } from "@/components/HomeScreen";
import { SendScreen } from "@/components/SendScreen";
import { ReceiveScreen } from "@/components/ReceiveScreen";
import { ObjectsScreen } from "@/components/ObjectsScreen";
import { ActivityScreen } from "@/components/ActivityScreen";
import { SettingsScreen } from "@/components/SettingsScreen";
import { SwapScreen } from "@/components/SwapScreen";
import { NftGallery } from "@/components/NftGallery";
import { NftDetail } from "@/components/NftDetail";
import { BuyScreen } from "@/components/BuyScreen";
import { EnergyDashboard } from "@/components/EnergyDashboard";
import { BatchRefresh } from "@/components/BatchRefresh";
import { GhostRecovery } from "@/components/GhostRecovery";
import { DecayForecasting } from "@/components/DecayForecasting";
import { SocialLogin } from "@/components/SocialLogin";
import { OnboardingTutorial } from "@/components/OnboardingTutorial";
import { WalletConnectScreen } from "@/components/WalletConnectScreen";
import { LedgerConnect } from "@/components/LedgerConnect";
import { BridgeScreen } from "@/components/BridgeScreen";
import { PluginStore } from "@/components/PluginStore";
import { AiAssistant } from "@/components/AiAssistant";
import { BackupRestoreScreen } from "@/components/BackupRestoreScreen";
import { PortfolioScreen } from "@/components/PortfolioScreen";
import { PatronageScreen } from "@/components/PatronageScreen";
import { RefreshPoolScreen } from "@/components/RefreshPoolScreen";
import { GovernanceScreen } from "@/components/GovernanceScreen";
import { DsnDetailsScreen } from "@/components/DsnBadge";
import { ShardScreen } from "@/components/ShardScreen";
import { ContactsScreen } from "@/components/ContactsScreen";
import { DaVerifyScreen } from "@/components/DaVerifyScreen";
import { TxToastContainer } from "@/components/TxToast";

export function App() {
  const { view, init, completeTutorial } = useWallet();

  useEffect(() => {
    init();
  }, [init]);

  // Lock-on-blur and lock-on-tab-close. The blur listener fires when
  // the popup itself loses focus (Chrome closes the popup as soon as
  // the user clicks outside it, so this is also the popup-close path
  // for non-tab-close cases). beforeunload covers the user explicitly
  // closing the popup or the browser tab. Both gate on the relevant
  // pref so existing users keep their previous behaviour.
  useEffect(() => {
    const onBlur = () => {
      const { preferences, isUnlocked, lock } = useWallet.getState();
      if (!isUnlocked) return;
      // `document.hasFocus()` returns false the moment the popup loses
      // focus — that's the desired trigger. Tab switches inside Chrome
      // never reach the popup's window because the popup is destroyed
      // when it loses focus, so this listener only fires for the
      // popup's own blur events.
      if (preferences.lockOnBlur && !document.hasFocus()) {
        lock();
      }
    };
    const onUnload = () => {
      const { preferences, isUnlocked, lock } = useWallet.getState();
      if (!isUnlocked) return;
      if (preferences.lockOnTabClose) {
        lock();
        // Best-effort: also flip the persisted lock flag the
        // background service-worker reads on next wake-up so the
        // wallet stays locked even before the popup re-mounts.
        try {
          if (typeof chrome !== "undefined" && chrome.storage?.local) {
            chrome.storage.local.set({ wallet_locked: true });
          }
        } catch {
          /* ignore */
        }
      }
    };
    window.addEventListener("blur", onBlur);
    window.addEventListener("beforeunload", onUnload);
    return () => {
      window.removeEventListener("blur", onBlur);
      window.removeEventListener("beforeunload", onUnload);
    };
  }, []);

  return (
    <>
      <AppView view={view} completeTutorial={completeTutorial} />
      {/* Mounted once at the root so toasts overlay every screen. */}
      <TxToastContainer />
    </>
  );
}

type ViewType = ReturnType<typeof useWallet.getState>["view"];

function AppView({ view, completeTutorial }: { view: ViewType; completeTutorial: () => void }) {
  switch (view) {
    case "locked":
      return <LockScreen />;
    case "create":
      return <CreateAccount />;
    case "import":
      return <ImportAccount />;
    case "home":
      return <HomeScreen />;
    case "send":
      return <SendScreen />;
    case "receive":
      return <ReceiveScreen />;
    case "objects":
      return <ObjectsScreen />;
    case "activity":
      return <ActivityScreen />;
    case "settings":
      return <SettingsScreen />;
    case "backup":
      return <BackupRestoreScreen />;
    case "portfolio":
      return <PortfolioScreen />;
    case "swap":
      return <SwapScreen />;
    case "nfts":
      return <NftGallery />;
    case "nft-detail":
      return <NftDetail />;
    case "buy":
      return <BuyScreen />;
    case "energy-dashboard":
      return <EnergyDashboard />;
    case "batch-refresh":
      return <BatchRefresh />;
    case "ghost-recovery":
      return <GhostRecovery />;
    case "decay-forecast":
      return <DecayForecasting />;
    case "social-login":
      // Simulated OAuth flow is dev-only — in prod fall back to the real
      // create flow until the backend is wired up. TODO real OAuth.
      return import.meta.env.DEV ? <SocialLogin /> : <CreateAccount />;
    case "tutorial":
      return <OnboardingTutorial onComplete={completeTutorial} />;
    case "walletconnect":
      return <WalletConnectScreen />;
    case "ledger":
      // TODO: enable when EvaporChain Ledger BOLOS app ships
      return import.meta.env.DEV ? <LedgerConnect /> : <HomeScreen />;
    case "bridge":
      return <BridgeScreen />;
    case "plugins":
      return <PluginStore />;
    case "ai-assistant":
      return <AiAssistant />;
    case "patronage":
      return <PatronageScreen />;
    case "refresh-pool":
      return <RefreshPoolScreen />;
    case "governance":
      return <GovernanceScreen />;
    case "dsn-details":
      return <DsnDetailsScreen />;
    case "shards":
      return <ShardScreen />;
    case "contacts":
      return <ContactsScreen />;
    case "da-verify":
      return <DaVerifyScreen />;
    default:
      return <LockScreen />;
  }
}

