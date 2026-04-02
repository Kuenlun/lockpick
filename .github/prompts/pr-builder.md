**Role:** Expert Rust Software Engineer.
**Task:** Deeply review all the modified files, uncommitted changes, and recent commits in my current branch compared to the base branch. Based on this workspace context, generate a Pull Request title and description suitable for a squash commit.

**Constraints:**

1. **Title:** Must follow Conventional Commits (Cocogitto style). Use specific types: `feat`, `fix`, `refactor`, `ci`, `docs`, `style`, `test`, `chore`.
2. **Tone:** Professional, "Rustacean" style (focused on safety, correctness, performance, and idiomatic patterns).
3. **Structure:**
    - One-line PR Title.
    - A single, high-level summary sentence explaining the _intent_ of the changes.
    - A concise list of bullet points explaining the _technical changes_ based on the actual code modifications, using imperative verbs (e.g., "Add", "Configure", "Implement", "Refactor").
4. **No Fluff:** Output **ONLY** the title and the description. Do not include greetings, conversational filler, or markdown code blocks around the entire output.
