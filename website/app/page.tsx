import Navbar from "@/components/Navbar";
import Hero from "@/components/Hero";
import Problem from "@/components/Problem";
import Solution from "@/components/Solution";
import Innovations from "@/components/Innovations";
import Architecture from "@/components/Architecture";
import Contracts from "@/components/Contracts";
import Numbers from "@/components/Numbers";
import Roadmap from "@/components/Roadmap";
import Waitlist from "@/components/Waitlist";
import Footer from "@/components/Footer";

export default function Home() {
  return (
    <>
      <Navbar />
      <main>
        <Hero />
        <Problem />
        <Solution />
        <Innovations />
        <Architecture />
        <Contracts />
        <Numbers />
        <Roadmap />
        <Waitlist />
      </main>
      <Footer />
    </>
  );
}
