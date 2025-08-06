# SystemJS Transpilation via Oxc

[Working on progress...]

Ported from [Babel's implementation](https://github.com/babel/babel/tree/main/packages/babel-plugin-transform-modules-systemjs).

Takeaways:

1. Several configurations not implemented yet.
2. Regarding named exports with specifiers, we export them in the pre-execute `_exports` first.
3. [IMPORTANT] The conversion from `VariableDeclaration` to `AssignmentExpression` uses a "hacked" mechanism that generates the code, eliminate the `var`, and parse it back. Further modification needed.
4. Not aligned with Babel yet, nor do tests exist.
5. If there's any problems, feel free to open an issue, or even better, a PR.
6. I'm entering 12th grade in China soon, so I may not have time to maintain this project. Contributions are welcome.
