// d3-force-3d ships no types. Its API is d3-force's plus a third axis, so reuse those types.
declare module "d3-force-3d" {
  import type { ForceY, Simulation, SimulationLinkDatum, SimulationNodeDatum } from "d3-force";
  export * from "d3-force";

  export interface Simulation3D<N extends SimulationNodeDatum, L extends SimulationLinkDatum<N> | undefined> extends Simulation<N, L> {
    numDimensions(): number;
    numDimensions(n: 1 | 2 | 3): this;
  }
  export function forceSimulation<N extends SimulationNodeDatum, L extends SimulationLinkDatum<N> = SimulationLinkDatum<N>>(nodes?: N[]): Simulation3D<N, L>;
  export function forceZ<N extends SimulationNodeDatum>(z?: number): ForceY<N>;
}
