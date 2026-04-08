import type { Metadata } from "next";
import { Inter } from "next/font/google";
import "./globals.css";

const inter = Inter({
  subsets: ["latin"],
  variable: "--font-inter",
  weight: ["300", "400", "500", "600", "700"],
});

export const metadata: Metadata = {
  metadataBase: new URL("https://evaporchain.com"),
  title: "EvaporChain — The Blockchain That Gets Lighter Over Time",
  description:
    "Thermodynamic state decay meets zero-knowledge proofs. A blockchain where unused state evaporates, the chain compresses to a constant-size proof, and everything is quantum-resistant from day one.",
  openGraph: {
    title: "EvaporChain — The Blockchain That Gets Lighter Over Time",
    description:
      "Thermodynamic state decay meets zero-knowledge proofs. The first blockchain that actually gets lighter over time.",
    type: "website",
    url: "https://evaporchain.com",
    images: [{ url: "/og-image.svg", width: 1200, height: 630 }],
  },
  twitter: {
    card: "summary_large_image",
    title: "EvaporChain — The Blockchain That Gets Lighter Over Time",
    description:
      "Thermodynamic state decay meets zero-knowledge proofs. The first blockchain that actually gets lighter over time.",
    images: ["/og-image.svg"],
  },
  icons: {
    icon: "/favicon.svg",
  },
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" className={`${inter.variable} h-full antialiased`}>
      <body className="min-h-full flex flex-col font-[family-name:var(--font-inter)]">
        {children}
      </body>
    </html>
  );
}
