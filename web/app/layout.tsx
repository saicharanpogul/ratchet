import type { Metadata } from "next";
import "./globals.css";
import { Header } from "../components/Header";
import { Footer } from "../components/Footer";

export const metadata: Metadata = {
  title: "ratchet — upgrade-safety for Solana programs",
  description:
    "Diff a candidate Anchor IDL against the deployed program and fail CI on changes that would silently corrupt on-chain state or break clients.",
  openGraph: {
    title: "ratchet",
    description: "Upgrade-safety checker for Solana programs.",
  },
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body className="min-h-screen flex flex-col">
        <Header />
        <main className="flex-1">{children}</main>
        <Footer />
      </body>
    </html>
  );
}
