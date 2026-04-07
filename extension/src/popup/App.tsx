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

export function App() {
  const { view, init, completeTutorial } = useWallet();

  useEffect(() => {
    init();
  }, [init]);

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
      return <SocialLogin />;
    case "tutorial":
      return <OnboardingTutorial onComplete={completeTutorial} />;
    case "walletconnect":
      return <WalletConnectScreen />;
    case "ledger":
      return <LedgerConnect />;
    case "bridge":
      return <BridgeScreen />;
    case "plugins":
      return <PluginStore />;
    case "ai-assistant":
      return <AiAssistant />;
    default:
      return <LockScreen />;
  }
}
