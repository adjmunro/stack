# Styleguide.md

## Communication & Writing
- Brevity is key! 
- Front-load the most important information.
- Keep things short and concise. That means use as short and few sentences as necessary to deliver the key information and nuances.
- Say what it does, not how it feels.
- Cut puffery, exposition, an LLM idiosyncrasies. 
  - Examples of LLM idiosyncrasies: over-used phrases like "load-bearing", "ground-truth", or "the smoking gun".
- Use bullet points when there are multiple key points, pieces of information, or logical conditions and branches.
- Use italics, bold, and font-colour to draw attention to key details (when possible).

### Conversations With Humans
- Use normal British English spelling, but with Oxford commas.
- Humans don't have time read an essay for every turn's feedback. 
- Some humans even switch between multiple LLM conversations frequenetly between turns.
- Don't keep bringing up things your human has dismissed, not even to say you didn't do it.
- A picture tells a thousand words: create richly formatted text; single web pages; and/or visualisations, pictures, and diagrams; to help communicate dense information quickly. Some information may be better suited to and easier to understand in these alternative formats over raw text.

### Documentation Comments
- Describe the external surface of the code:
  - What comes in;
  - What goes out;
  - How to use it;
  - Exceptions thrown;
  - Any caveats & nuances important for the surface consumer to know about (in case they need to work around it).
- Describe the current state, _not_ how or why it got that way. 
- Historical and chronological notes belong in the git commit messages, not here.
- We do not care _why_, unless that "why" is related to the present codebase.
- Caveats & nuances that have no bearing on the consumer belong in inline comments, not here.

### Inline Comments
- Describe the non-obvious, that is difficult or impossible to infer from the code. For example, if the code:
  - Is complicated and obscure; 
  - Contains many layers of nested and/or branching logic;
  - Uses human-hostile operations like bit manipulations; or 
  - Is affected by behaviour or side effects hidden behind another object or function call.
- Describe the current state, _not_ how or why it got that way (except when related to an unresolved known bug or TODO). 
- Historical and chronological notes belong in the git commit messages, not here.
- We do not care _why_, unless that "why" is related to the present codebase.

### Git Commit Messages
- Be extremely brief, these sometimes show up in release notes.
- Generally, treat commits as archeological artefacts for posterity: only keep the information important for future context, debugging, and/or caveats & nuance.
- Usually, commits are most interested in cause and effect or the reason for the change.

### Markdown
- Don't artificially chop lines according to line length - I have soft-wrap enabled for a reason and adding newlines where they don't belong just makes things look weird.

## Code
- Always use the minimum viable visibility modifier.

### Program Architecture, Habits, & System Design
- Pay attention to architecture, using appropriate design patterns where applicable. 
  - https://refactoring.guru/design-patterns/catalog
- Appreciate the expertise of Dave Farley, Martin Fowler, Michael Feathers, and Bob Martin.
