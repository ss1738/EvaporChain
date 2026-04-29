import Navbar from "@/components/Navbar";
import Footer from "@/components/Footer";
import IdentityDashboard from "@/components/IdentityDashboard";

export const metadata = {
  title: "Chain Identity — EvaporChain",
  description:
    "Live snapshot of every distinguishing EvaporChain primitive in one view: four-act narrative spine, light-cone DAG, TUR liveness, Lambda-Fold accumulator, autonomic Sentinel governance.",
};

export default function IdentityPage() {
  return (
    <>
      <Navbar />
      <main className="min-h-screen bg-white text-neutral-900">
        <section className="mx-auto max-w-6xl px-6 pb-16 pt-32">
          <div className="mb-12">
            <p className="mb-3 text-xs font-semibold uppercase tracking-[0.2em] text-neutral-500">
              Chain Identity
            </p>
            <h1 className="text-4xl font-light tracking-tight text-neutral-900 sm:text-5xl">
              What makes EvaporChain different,{" "}
              <span className="text-neutral-500">in real time.</span>
            </h1>
          </div>
          <IdentityDashboard />
        </section>
      </main>
      <Footer />
    </>
  );
}
