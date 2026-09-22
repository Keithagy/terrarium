import { invoke } from "@tauri-apps/api/core";

export interface User { id: number; name: string }

export async function fetchUsers(): Promise<User[]> {
  const res = await fetch("/api/users");
  return res.json();
}

export async function fetchUser(id: number): Promise<User> {
  const res = await fetch(`/api/users/${id}`);
  return res.json();
}

export const scanRepo = (path: string) => invoke<string>("scan_repo", { path });

export async function fetchReport(): Promise<unknown> {
  const res = await fetch("/api/reports");
  return res.json();
}
