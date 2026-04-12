import React, { useMemo, useState } from "react";
import { motion } from "framer-motion";
import { Sparkles, Cpu, LayoutGrid, Gauge, ArrowRight, ShieldCheck, Wand2, Boxes, TerminalSquare } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";

const features = [
  {
    title: "Renderer",
    value: "wgpu · GPU accelerated",
    icon: Cpu,
    hint: "Zero-jank motion, buttery transitions, native-feeling compositing.",
  },
  {
    title: "Layout",
    value: "Flexbox via taffy",
    icon: LayoutGrid,
    hint: "Predictable layout primitives with modern spacing and adaptive grids.",
  },
  {
    title: "Target",
    value: "120fps desktop",
    icon: Gauge,
    hint: "Tuned for snappy interactions, micro-animations, and absurd responsiveness.",
  },
];

const pills = ["Fast", "Composable", "Animated", "Typed", "Actually usable"];

function Aurora() {
  return (
    <div className="pointer-events-none absolute inset-0 overflow-hidden">
      <motion.div
        className="absolute -top-24 left-[-10%] h-72 w-72 rounded-full bg-cyan-500/20 blur-3xl"
        animate={{ x: [0, 80, 0], y: [0, 20, 0], scale: [1, 1.15, 1] }}
        transition={{ duration: 14, repeat: Infinity, ease: "easeInOut" }}
      />
      <motion.div
        className="absolute right-[-5%] top-24 h-80 w-80 rounded-full bg-fuchsia-500/15 blur-3xl"
        animate={{ x: [0, -60, 0], y: [0, 40, 0], scale: [1.1, 1, 1.1] }}
        transition={{ duration: 16, repeat: Infinity, ease: "easeInOut" }}
      />
      <motion.div
        className="absolute bottom-[-8%] left-1/3 h-72 w-72 rounded-full bg-emerald-400/10 blur-3xl"
        animate={{ x: [0, -30, 0], y: [0, -30, 0] }}
        transition={{ duration: 12, repeat: Infinity, ease: "easeInOut" }}
      />
      <div className="absolute inset-0 bg-[radial-gradient(circle_at_top,rgba(56,189,248,0.10),transparent_30%),radial-gradient(circle_at_80%_20%,rgba(217,70,239,0.08),transparent_28%),linear-gradient(to_bottom,rgba(255,255,255,0.02),transparent_20%)]" />
      <div className="absolute inset-0 opacity-[0.08] [background-image:linear-gradient(rgba(255,255,255,.15)_1px,transparent_1px),linear-gradient(90deg,rgba(255,255,255,.15)_1px,transparent_1px)] [background-size:72px_72px]" />
      <div className="absolute inset-0 bg-[linear-gradient(to_bottom,transparent,rgba(2,6,23,.5),rgba(2,6,23,.95))]" />
    </div>
  );
}

function StatCard({ feature, index }) {
  const Icon = feature.icon;
  return (
    <motion.div
      initial={{ opacity: 0, y: 20 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ delay: 0.15 + index * 0.08, duration: 0.55 }}
      whileHover={{ y: -4, scale: 1.01 }}
      className="group"
    >
      <Card className="relative h-full overflow-hidden rounded-3xl border-white/10 bg-white/[0.04] shadow-2xl shadow-cyan-950/20 backdrop-blur-xl transition-colors duration-300 group-hover:border-cyan-400/30 group-hover:bg-white/[0.06]">
        <div className="absolute inset-0 bg-[radial-gradient(circle_at_top_left,rgba(34,211,238,0.10),transparent_30%)] opacity-0 transition-opacity duration-300 group-hover:opacity-100" />
        <CardContent className="relative p-6">
          <div className="mb-4 flex items-center justify-between">
            <div>
              <p className="text-sm text-slate-400">{feature.title}</p>
              <h3 className="mt-1 text-2xl font-semibold tracking-tight text-white">{feature.value}</h3>
            </div>
            <div className="rounded-2xl border border-white/10 bg-white/5 p-3 text-cyan-300">
              <Icon className="h-5 w-5" />
            </div>
          </div>
          <p className="text-sm leading-6 text-slate-400">{feature.hint}</p>
        </CardContent>
      </Card>
    </motion.div>
  );
}

export default function UnshitFrameworkLanding() {
  const [name, setName] = useState("unshit framework");
  const launchLabel = useMemo(() => {
    const cleaned = name.trim();
    return cleaned.length ? cleaned : "your next thing";
  }, [name]);

  return (
    <div className="min-h-screen overflow-hidden bg-slate-950 text-white">
      <Aurora />

      <main className="relative mx-auto flex min-h-screen w-full max-w-7xl flex-col px-6 pb-12 pt-6 sm:px-8 lg:px-10">
        <motion.div
          initial={{ opacity: 0, y: 18 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.65 }}
          className="rounded-[32px] border border-white/10 bg-slate-900/50 p-4 shadow-2xl shadow-black/30 backdrop-blur-2xl sm:p-5"
        >
          <div className="rounded-[26px] border border-white/10 bg-gradient-to-r from-white/[0.04] to-white/[0.02] p-4 sm:p-5">
            <div className="mb-5 flex flex-wrap items-center gap-2">
              <Badge className="rounded-full border border-cyan-400/20 bg-cyan-400/10 px-3 py-1 text-cyan-200 hover:bg-cyan-400/10">
                <Sparkles className="mr-1.5 h-3.5 w-3.5" />
                Experimental UI Runtime
              </Badge>
              {pills.map((pill) => (
                <Badge
                  key={pill}
                  variant="secondary"
                  className="rounded-full border border-white/10 bg-white/5 px-3 py-1 text-slate-300"
                >
                  {pill}
                </Badge>
              ))}
            </div>

            <div className="grid gap-8 lg:grid-cols-[1.3fr_0.7fr] lg:items-end">
              <div>
                <div className="mb-6 rounded-3xl border border-white/10 bg-black/20 p-3 shadow-inner shadow-black/20">
                  <Input
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    className="h-14 border-0 bg-transparent px-3 text-2xl font-semibold tracking-tight text-cyan-300 shadow-none ring-0 placeholder:text-slate-500 focus-visible:ring-0"
                    placeholder="name your framework"
                  />
                </div>

                <motion.h1
                  initial={{ opacity: 0, y: 12 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ delay: 0.12, duration: 0.6 }}
                  className="max-w-3xl text-4xl font-semibold tracking-tight text-white sm:text-5xl lg:text-6xl"
                >
                  Build products that feel <span className="bg-gradient-to-r from-cyan-300 via-sky-400 to-fuchsia-400 bg-clip-text text-transparent">impossibly smooth</span>.
                </motion.h1>

                <motion.p
                  initial={{ opacity: 0, y: 12 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ delay: 0.2, duration: 0.6 }}
                  className="mt-5 max-w-2xl text-base leading-7 text-slate-300 sm:text-lg"
                >
                  {launchLabel} is a modern runtime-inspired interface concept with GPU-first rendering, polished motion,
                  layered depth, and a design system that doesn’t look like a developer placeholder.
                </motion.p>

                <motion.div
                  initial={{ opacity: 0, y: 12 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ delay: 0.28, duration: 0.6 }}
                  className="mt-8 flex flex-wrap gap-3"
                >
                  <Button className="group h-12 rounded-2xl bg-emerald-500 px-5 text-sm font-medium text-white shadow-lg shadow-emerald-900/30 transition-transform hover:scale-[1.02] hover:bg-emerald-400">
                    Build something
                    <ArrowRight className="ml-2 h-4 w-4 transition-transform group-hover:translate-x-0.5" />
                  </Button>
                  <Button
                    variant="outline"
                    className="h-12 rounded-2xl border-red-400/30 bg-red-500/10 px-5 text-sm font-medium text-red-200 backdrop-blur-md hover:bg-red-500/20"
                  >
                    Unshit the world
                  </Button>
                </motion.div>
              </div>

              <motion.div
                initial={{ opacity: 0, scale: 0.98 }}
                animate={{ opacity: 1, scale: 1 }}
                transition={{ delay: 0.15, duration: 0.65 }}
                className="relative"
              >
                <div className="absolute -inset-2 rounded-[32px] bg-gradient-to-br from-cyan-500/15 via-sky-400/10 to-fuchsia-500/15 blur-2xl" />
                <Card className="relative overflow-hidden rounded-[28px] border-white/10 bg-slate-950/70 shadow-2xl backdrop-blur-xl">
                  <div className="absolute inset-0 bg-[radial-gradient(circle_at_top_right,rgba(34,211,238,0.12),transparent_30%)]" />
                  <CardContent className="relative p-6">
                    <div className="mb-4 flex items-center gap-2 text-slate-400">
                      <div className="h-2.5 w-2.5 rounded-full bg-rose-400/70" />
                      <div className="h-2.5 w-2.5 rounded-full bg-amber-400/70" />
                      <div className="h-2.5 w-2.5 rounded-full bg-emerald-400/70" />
                      <span className="ml-2 text-xs uppercase tracking-[0.22em]">live preview</span>
                    </div>

                    <div className="rounded-3xl border border-white/10 bg-white/[0.03] p-5">
                      <div className="mb-5 flex items-center justify-between">
                        <div>
                          <p className="text-xs uppercase tracking-[0.22em] text-slate-500">Runtime health</p>
                          <p className="mt-2 text-2xl font-semibold text-white">Nominal</p>
                        </div>
                        <div className="rounded-2xl border border-emerald-400/20 bg-emerald-400/10 px-3 py-2 text-sm text-emerald-300">
                          120 fps
                        </div>
                      </div>

                      <div className="grid gap-3">
                        {[
                          { icon: Boxes, label: "Composable primitives", value: "24 ready" },
                          { icon: Wand2, label: "Motion presets", value: "11 active" },
                          { icon: ShieldCheck, label: "Type safety", value: "strict" },
                          { icon: TerminalSquare, label: "DX score", value: "pleasant" },
                        ].map((item, i) => {
                          const Icon = item.icon;
                          return (
                            <motion.div
                              key={item.label}
                              initial={{ opacity: 0, x: 8 }}
                              animate={{ opacity: 1, x: 0 }}
                              transition={{ delay: 0.35 + i * 0.08, duration: 0.45 }}
                              className="flex items-center justify-between rounded-2xl border border-white/10 bg-black/20 px-4 py-3"
                            >
                              <div className="flex items-center gap-3">
                                <div className="rounded-xl border border-white/10 bg-white/5 p-2 text-cyan-300">
                                  <Icon className="h-4 w-4" />
                                </div>
                                <span className="text-sm text-slate-300">{item.label}</span>
                              </div>
                              <span className="text-sm font-medium text-white">{item.value}</span>
                            </motion.div>
                          );
                        })}
                      </div>
                    </div>
                  </CardContent>
                </Card>
              </motion.div>
            </div>
          </div>
        </motion.div>

        <section className="mt-8 grid gap-4 md:grid-cols-3">
          {features.map((feature, index) => (
            <StatCard key={feature.title} feature={feature} index={index} />
          ))}
        </section>
      </main>
    </div>
  );
}
