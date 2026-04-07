import { useEffect } from "react";
import { useWallet } from "@/hooks/useWallet";
import { LockScreen } from "@/components/LockScreen";
import { CreateAccount } from "@/components/CreateAccount";
import { HomeScreen } from "@/components/HomeScreen";
import { SendScreen } from "@/components/SendScreen";
import { ReceiveScreen } from "@/components/ReceiveScreen";
import { ObjectsScreen } from "@/components/ObjectsScreen";
import { SettingsScreen } from "@/components/SettingsScreen";

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
    case "home":
      return <HomeScreen />;
    case "send":
      return <SendScreen />;
    case "receive":
      return <ReceiveScreen />;
    case "objects":
      return <ObjectsScreen />;
    case "settings":
      return <SettingsScreen />;
    case "activity":
      // TODO: Activity screen
      return <HomeScreen />;
    default:
      return <LockScreen />;
  }
}
