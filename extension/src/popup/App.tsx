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

export function App() {
  const { view, init } = useWallet();

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
    default:
      return <LockScreen />;
  }
}
