import type { User } from "./api";

export function render(users: User[]) {
  const el = document.getElementById("app");
  if (el) el.textContent = users.map((u) => u.name).join(", ");
}
