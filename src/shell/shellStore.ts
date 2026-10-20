import { create } from 'zustand';

interface ShellState {
  startOpen: boolean;
  toggleStart: () => void;
  closeStart: () => void;
}

export const useShell = create<ShellState>((set) => ({
  startOpen: false,
  toggleStart: () => set((s) => ({ startOpen: !s.startOpen })),
  closeStart: () => set({ startOpen: false }),
}));
