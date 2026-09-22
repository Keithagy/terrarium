import { fetchUsers, scanRepo } from "./api";
import { render } from "./view";

export async function main() {
  const users = await fetchUsers();
  render(users);
  await scanRepo(import.meta.env.VITE_REPO_PATH);
}

main();
