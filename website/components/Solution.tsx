"use client";

import { motion } from "framer-motion";

const steps = [
  {
    title: "CREATE",
    label: "Object is created with energy",
    energy: 100,
    color: "#22c55e",
    opacity: 1,
  },
  {
    title: "DECAY",
    label: "Energy depletes each epoch",
    energy: 45,
    color: "#f59e0b",
    opacity: 0.6,
  },
  {
    title: "EVAPORATE",
    label: "Object evaporates. Ghost record remains.",
    energy: 5,
    color: "#ef4444",
    opacity: 0.15,
  },
];

function Orb({ energy, color, opacity, index }: { energy: number; color: string; opacity: number; index: number }) {
  return (
    <motion.div
      initial={{ opacity: 0, scale: 0.8 }}
      whileInView={{ opacity: 1, scale: 1 }}
      viewport={{ once: true }}
      transition={{ duration: 0.6, delay: index * 0.2 }}
      className="flex flex-col items-center"
    >
      <div className="relative w-20 h-20 mb-4">
        <div
          className="absolute inset-0 rounded-full"
          style={{
            background: `radial-gradient(circle, ${color}${Math.round(opacity * 255).toString(16).padStart(2, '0')}, transparent 70%)`,
            filter: `blur(${8 - opacity * 6}px)`,
          }}
        />
        <div
          className="absolute inset-2 rounded-full"
          style={{
            background: `radial-gradient(circle at 40% 35%, ${color}, transparent)`,
            opacity: opacity,
          }}
        />
        {index === 2 && (
          <>
            {[...Array(5)].map((_, i) => (
              <motion.div
                key={i}
                className="absolute w-1 h-1 rounded-full"
                style={{
                  background: color,
                  left: `${30 + i * 10}%`,
                  bottom: "60%",
                }}
                animate={{
                  y: [-10, -40],
                  opacity: [0.5, 0],
                }}
                transition={{
                  duration: 2,
                  delay: i * 0.3,
                  repeat: Infinity,
                  ease: "easeOut",
                }}
              />
            ))}
          </>
        )}
      </div>

      <div className="w-32 h-2 rounded-full bg-white/5 overflow-hidden mb-3">
        <motion.div
          className="h-full rounded-full"
          style={{ background: color }}
          initial={{ width: 0 }}
          whileInView={{ width: `${energy}%` }}
          viewport={{ once: true }}
          transition={{ duration: 1, delay: 0.3 + index * 0.2 }}
        />
      </div>
    </motion.div>
  );
}

export default function Solution() {
  return (
    <section id="solution" className="py-32 px-6 bg-[#0c0c14]">
      <div className="max-w-5xl mx-auto">
        <motion.div
          initial={{ opacity: 0, y: 30 }}
          whileInView={{ opacity: 1, y: 0 }}
          viewport={{ once: true }}
          transition={{ duration: 0.6 }}
          className="text-center"
        >
          <h2 className="text-3xl sm:text-4xl font-bold gradient-text inline-block">
            What If State Had a Lifespan?
          </h2>
          <p className="mt-4 text-text-secondary text-lg max-w-2xl mx-auto">
            EvaporChain introduces thermodynamic state decay — objects have
            energy that depletes over time.
          </p>
        </motion.div>

        <div className="mt-20 flex flex-col md:flex-row items-center justify-center gap-8 md:gap-12">
          {steps.map((step, i) => (
            <div key={step.title} className="flex items-center gap-8 md:gap-12">
              <div className="text-center">
                <Orb
                  energy={step.energy}
                  color={step.color}
                  opacity={step.opacity}
                  index={i}
                />
                <div className="text-xs font-semibold tracking-wider text-text-muted uppercase mb-1">
                  {step.title}
                </div>
                <div className="text-sm text-text-secondary max-w-[160px]">
                  {step.label}
                </div>
              </div>

              {i < steps.length - 1 && (
                <div className="hidden md:block">
                  <motion.div
                    initial={{ opacity: 0, scaleX: 0 }}
                    whileInView={{ opacity: 1, scaleX: 1 }}
                    viewport={{ once: true }}
                    transition={{ duration: 0.5, delay: 0.5 + i * 0.2 }}
                    className="w-16 h-px origin-left"
                    style={{
                      background: `linear-gradient(90deg, ${step.color}40, ${steps[i + 1].color}40)`,
                    }}
                  />
                </div>
              )}
            </div>
          ))}
        </div>

        <motion.p
          initial={{ opacity: 0 }}
          whileInView={{ opacity: 1 }}
          viewport={{ once: true }}
          transition={{ duration: 0.6, delay: 0.8 }}
          className="mt-16 text-center text-text-secondary italic max-w-2xl mx-auto"
        >
          No rent. No governance votes. No cleanup. Objects decay like physical
          matter — automatically, inevitably, beautifully.
        </motion.p>
      </div>
    </section>
  );
}
