How to Create Your Own TypeScript Library

If you want to build and publish a custom TypeScript package, follow this core configuration structure: \[[1](https://dev.to/shelner/how-to-create-a-simple-library-using-typescript-j7o), [2](https://gist.github.com/rsp/f7d6aec4f2bbac3de4bc3f88d871cc70)\]

**1\. Configure the tsconfig.json \[[1](https://techwize.com/blog/typescript-for-enhanced-type-safety-and-maintainability-in-react-projects)\]**

Your TypeScript configuration requires special parameters to correctly export type definitions (.d.ts files) alongside your compiled code: \[[1](https://www.tsmean.com/articles/how-to-write-a-typescript-library), [2](https://www.reddit.com/r/typescript/comments/7oxypp/how_do_you_go_about_using_typescript_in_a_library/), [3](https://gist.github.com/rsp/f7d6aec4f2bbac3de4bc3f88d871cc70)\]

json  
{  
  "compilerOptions": {  
    "target": "ES2022",  
    "module": "NodeNext",  
    "moduleResolution": "NodeNext",  
    "declaration": true,            // Generates the .d.ts type files  
    "declarationMap": true,         // Maps type files back to source code  
    "outDir": "./dist",             // Redirects compiled files  
    "strict": true                  // Guarantees maximum type safety  
  },  
  "include": \["src/\*\*/\*"\]  
}

Use code with caution.

**2\. Configure package.json Fields \[[1](https://medium.com/@rifantechguy55/how-to-develop-a-typescript-library-ade8d329636)\]**

To support both modern ES Modules (ESM) and older CommonJS (CJS) setups, leverage conditional exports: \[[1](https://www.youtube.com/watch?v=vRmLTZyq57U), [2](https://www.youtube.com/watch?v=OnBimwT5jOE&t=21), [3](https://dev.to/shelner/how-to-create-a-simple-library-using-typescript-j7o)\]

json  
{  
  "name": "your-library-name",  
  "version": "1.0.0",  
  "main": "./dist/index.js",  
  "types": "./dist/index.d.ts",  
  "exports": {  
    "types": "./dist/index.d.ts",  
    "import": "./dist/index.js"  
  },  
  "files": \[  
    "dist"  
  \],  
  "scripts": {  
    "build": "tsc"  
  }  
}

Use code with caution.

**3\. Streamlined Tooling Alternatives**

Instead of configuring the TypeScript Compiler (tsc) manually, many modern developers utilize high-performance zero-config bundlers: \[[1](https://www.reddit.com/r/typescript/comments/gzu67w/creating_typescript_libraries_packages/), [2](https://dev.to/shelner/how-to-create-a-simple-library-using-typescript-j7o)\]

* [tsup](https://dev.to/shelner/how-to-create-a-simple-library-using-typescript-j7o): Powered by esbuild; instantly outputs CJS, ESM, and types with zero setup.  
* [microbundle](https://github.com/developit/microbundle): Powered by Rollup; ideal for producing extremely compact, universal packages. \[[1](https://tsup.egoist.dev/), [2](https://github.com/officialrajdeepsingh/awesome-nextjs), [3](https://www.reddit.com/r/typescript/comments/yeq0mi/best_builder_for_typescript_library/), [4](https://www.reddit.com/r/typescript/comments/gzu67w/creating_typescript_libraries_packages/), [5](https://dev.to/shelner/how-to-create-a-simple-library-using-typescript-j7o)\]

Are you looking to **install and use** a specific type of library, or are you trying to **build and publish** your own npm package? Let me know your exact goal so I can provide the right code examples or commands.

![][image1]![][image2]![][image3]  
27 sites

* Creating a library: best practices : r/typescript \- Reddit  
  I want to create a js/ts library, it will be mostly a library with a core package to communicate with a LLM and several components...  
  ![][image4]  
  ![][image5]  
  Reddit  
* My Favorite TypeScript Library Just Got So Much Better  
  i love Effect it feels like an entirely new language built on top of TypeScript that fixes error handling brings in really good fu...  
  ![Related video thumbnail][image6]  
  25:21  
  ![][image7]  
  YouTube·Ben Davis  
* Creating and using a Node library with TypeScript · GitHub  
  To create a TypeScript library, you can follow these steps: 1\. Find a free name on npm 2\. Create a GitHub repo 3\. Create a \`packag...  
  ![][image8]  
  ![][image9]  
  GitHub

Show all  
I'm trying to create a typescript library with neon which integrates with a rust library in a separate repo to keep the rust library and ts library decoupled. I'm trying to come up with a name for the repo \- solx-core is the rust library repo. So we'd need to create rust libraries for the neon bindings which wrap the existing rust library, and then create typescript libraries which surface the neon packages. Is that correct?

Yes, your planned architectural approach is **100% correct**.

To maintain strict decoupling, you need an intermediate "binding layer" that bridges your pure Rust engine with the Node.js runtime.

Here is exactly how the architecture maps out, along with naming conventions and repository recommendations for your project.

The 3-Tier Decoupled Architecture

\[ Your Application \]  
         │  
         ▼  
 1\. TypeScript Library  (NPM Package / Consumable Wrapper)  
         │  
         ▼  
 2\. Neon Binding Layer  (Rust N-API Bridge)  
         │  
         ▼  
 3\. solx-core           (Pure Rust Library Crate)

1. **solx-core (Existing Repo)**: A pure Rust crate containing your core domain logic, algorithms, and data structures. It knows absolutely nothing about Node.js, JavaScript, or Neon.  
2. **Neon Binding Layer**: A specialized Rust project that imports solx-core as a dependency. It uses the neon crate to safely map JavaScript arguments to Rust types, call your core functions, and return JavaScript-compatible values. \[[1](https://docs.rs/neon)\]  
3. **TypeScript Library**: The consumer-facing package that exports developer-friendly TypeScript functions, provides strict IntelliSense typing, handles any JS-side asynchronous wrapping, and exports the final binary.

---

Repository & Naming Strategies

Since you want to keep the TypeScript ecosystem isolated from your pure Rust ecosystem, you have two primary options for structuring the remaining repositories.

**Option A: The Unified JS Repo (Highly Recommended)**

Instead of creating separate repos for the Neon bindings and the TS definitions, combine them into **one repository** utilizing a monorepo setup (e.g., using npm workspaces or pnpm workspaces). \[[1](https://medium.com/@harshverma04111989/different-flavors-of-monorepos-521df49131d0)\]

* **Suggested Repo Name**: solx-js or solx-node  
* **Internal Structure**:  
  * packages/solx-bindings/ (The Neon crate containing Rust bridge code)  
  * packages/solx-sdk/ (The TypeScript library surfacing the APIs)  
* **Why this works best**: Neon packages natively bundle their Rust workspace (/src and Cargo.toml) alongside a companion package.json. Keeping the Neon code and the TS types in the same repo ensures your TypeScript typings never drift out of sync with your Rust-to-JS compilation logic.

**Option B: Completely Separate Repositories**

If you want total physical separation of every single layer:

1. **Neon Binding Repo**: solx-bindings (or solx-neon)  
   * *Purpose*: Exclusively compiles the native .node binary and exposes raw, low-level JavaScript bindings.  
2. **TypeScript Repo**: solx-sdk (or solx-ts)  
   * *Purpose*: Imports solx-bindings as a dependency, polishes the API, and ships clean TypeScript definitions to the end-user.

---

To help you map out the next implementation steps, let me know:

* Will this library be used **internally** by your team, or published as an **open-source public NPM package**?  
* Do you need to target **cross-compilation** (e.g., building binaries for Linux, macOS, and Windows) using GitHub Actions?

2 sites

* neon \- Rust  
  The Neon crate provides bindings for writing Node. js addons (i.e., dynamically-loaded binary modules) with a safe and fast Rust A...  
    
  Docs.rs  
* Monorepos and its flavors. Package-based VS Integrated | by Harsh Verma  
  Feb 24, 2023 — Quick Overview on Monorepo ( mono repo ) \- Maintains all those similar things in all the web apps (i.e. in short duplicating code)  
  ![][image10]  
    
  Medium  
  It will be an open source npm package, and we would need to set up cross compilation if possible.

Cross-compilation is highly recommended for an open-source library, as it ensures users don't need a local Rust toolchain or C++ build tools installed just to run your package. \[[1](https://adaptivesupport.amd.com/s/question/0D5KZ00000Ivp2L0AR/-info-post-crosscompiling-vitis-ai-applications-in-host-pc?language=en_US), [2](https://reemus.dev/tldr/rust-cross-compilation-github-actions)\]

The Neon project features an officially supported initialization method via **create-neon** explicitly designed to scaffold cross-compilation architectures out of the box. \[[1](https://www.npmjs.com/package/create-neon)\]

1\. The Cross-Compilation Strategy (N-API Sub-packages)

Because Neon uses Node-API (N-API), you do not need a separate build for every Node.js version—just a build for every major Operating System and CPU architecture combination. \[[1](https://npmjs.com/package/prebuildify), [2](https://github.com/prebuild/prebuild-install/blob/master/README.md)\]

The industry-standard approach for an open-source package is the **Multi-package / Monorepo deployment**. You publish a main wrapper package alongside a set of targeted architecture sub-packages:

* solx-sdk (The main TypeScript package the user installs)  
* solx-sdk-win32-x64 (Precompiled Windows binary)  
* solx-sdk-linux-x64 (Precompiled Linux binary)  
* solx-sdk-darwin-arm64 (Precompiled Apple Silicon binary)

When a user runs npm install solx-sdk, npm uses the optionalDependencies block in package.json to automatically download *only* the specific sub-package binary needed for their active computer platform.

2\. Quick Scaffolding with create-neon

To build this structure without manually wiring up complex multi-package linkages, use the official Neon template generator inside your new solx-js repository: \[[1](https://www.npmjs.com/package/create-neon)\]

bash  
npm init neon@latest \-- \--lib solx-sdk

Use code with caution.

*(Note: The double dashes \-- pass parameters correctly through npm init).* \[[1](https://www.npmjs.com/package/create-neon)\]

This command automatically generates a modern, production-ready workspace structure that includes:

* Your **TypeScript source code wrapper**.  
* Your **Neon Rust crate** (Cargo.toml and /src) pointing to your decoupled solx-core via standard \[dependencies\].  
* A complete pre-configured continuous integration folder containing a .github/workflows/ci.yml file. \[[1](https://www.youtube.com/watch?v=E7Dvkx_Xr2k&t=699), [2](https://www.youtube.com/watch?v=jkC4vik8__k), [3](https://www.npmjs.com/package/create-neon), [4](https://github.com/microsoft/TypeScript/issues/17611)\]

3\. Automating Builds with GitHub Actions

The GitHub Actions workflow template provided by Neon handles the heavy lifting of cross-compilation. When a new version tag is pushed, it spins up a multi-platform matrix:

yaml  
\# A simplified view of how Neon's generated workflow handles cross-compilation  
strategy:  
  matrix:  
    os: \[ubuntu-latest, macos-latest, windows-latest\]  
    target: \[x86\_64, aarch64\]

Use code with caution.

1. **Native Hosts**: It leverages GitHub’s native macOS runners to build the darwin-x64 and darwin-arm64 binaries, and Windows runners for win32 binaries.  
2. **Linux Cross-Compilation**: To compile for multiple Linux targets (like ARM64, musl, etc.) from an Ubuntu runner, the workflow uses **cross-rs** (a Docker-based container isolation system for cross-compiling Rust projects without manually mapping C-linkers). \[[1](https://www.reddit.com/r/rust/comments/o8z614/cross_compiling_rust_binaries_with_github_actions/), [2](https://github.com/marketplace/actions/build-optimized-rust-binary)\]

How the decoupling looks in your Cargo.toml

Inside your freshly generated Neon crate folder (/src/native/Cargo.toml), you will simply point to your independent solx-core repo:

toml  
\[dependencies\]  
neon \= "1.0" \# Or current stable version  
solx-core \= { git \= "https://github.com", branch \= "main" }

[image1]: <data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABQAAAAUCAMAAAC6V+0/AAADAFBMVEVHcEz/RQD/RgD/RQD/RQD/RQD/RQD/RQD/RQD/RgD/RgD/RQD+///8/v7/QQD5+/zK1d3N2eD1+Pnl7fDr8vQDCArw9vfT3eMLGB3f6eza4uf+Ug//YAL/OwD5dU3tOwr01s71jG76wKz6YTL3p4787OfSxMPPLRU9RUezQiolLTDarKObHRekXlutnJ5xd3mRiYxWXmIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAACA2YoWAAAAC3RSTlMAyTFF3l+2jvEXCRH5QN8AAADLSURBVHheXZHrDsIwCIU5gJ23GJclJmp8/7czJmbOy9YJbc3U86MtHxQKBSUtOG2xSxvSusouU/uBE9peEsY3M2N0mnNlLdcQ8QNPgaebKAa/iVKXSGgop8jG6kr2QIzAXg613caqvGrS6IX+ILwQhQDindvNjhBCgned0ebpsH8S68tqmkOYhxamO3GghwEF9csIDqp6bGns/Ek+l3mUqhFpzkE3Zna5d/l0YAmtd+vVso4RNepwjdlhkZWWsKw0ul+Whqz0zcp3vAF4uC3AZ1qPKgAAAABJRU5ErkJggg==>

[image2]: <data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABQAAAAUCAMAAAC6V+0/AAADAFBMVEVHcEz/ADP/ADP/ADP/ADP/ADP/ADP/ADP/ADP/////hpT/2uH/Smj/s8D/JEv/b4cAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADc5neqAAAACHRSTlMAQB+z1GXri3bBstgAAABRSURBVHhepc9BDoAgDETRX2OI9z8tcWGBRLC13RhnRV8YUuBXBMrm5Do7Ho5GqkQDX72zUDJEnqOp67rs3pzDbkw14BRM3Vi+Z/5NgtbX/DkNAx8KJg11sZkAAAAASUVORK5CYII=>

[image3]: <data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABQAAAAUCAMAAAC6V+0/AAADAFBMVEX///8kKS74+PgsMTY+Q0f8/Pzw8PGRk5ZNUVXr6+xDSEzCxMUpLjLV1tYuMzc/Q0i3ubuUl5lbX2NTV1uLjpFKTlKxs7Xh4uOFh4rn5+h3en1vcnZ9gIPc3d6/wcJzd3o3PECmqKpjZ2rOz9CfoaQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABdy0YeAAAAyUlEQVR4Xk2QjQ6CMAyE77YBKiooUTT+Jr7/wxljMBjEdgznJWvXr13XzcLLFJikpn72Q6jK5gyyq4BObkSq2R5wgO3iGaDxdkdukc+4qcgcZzITKP0WclMKpOJrcikRtfInqWSCUmwaodbcjAXyNsKPAXrb9nDvCLFu0ElCx4q6yzIl8PiHMrQzyT8ZZPx1HF+MlUbScr48F7JTdPCvt7Bg0yELQ3lXvMRcWY6ntdBpU3SX+ybTX/CyVdgcQ8+E3I1J0bQe3BB9AcF8GbhO3b0SAAAAAElFTkSuQmCC>

[image4]: <data:image/png;base64,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>

[image5]: <data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAMAAAAoLQ9TAAADAFBMVEVHcEz/RQD/RAD/RAD/RQD/RQD/RQD/RQD/PABHcEz/RQD/////QAD/OAD1///o9/v7bEriuLT+8ez/4tvtxL31e2PS6fH+UBr0LwD/XjTf7/P/MQD/sqP/Wi78lX/zl4Xn1tjSzdDokoLt0c7/XgCmrrDO4+13ensAAACFjI6DIwDCNgVjTEf/HwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA7tfP3AAAACnRSTlMA6NrQfyCilP4AYqLPiwAAAMVJREFUeF4lTstuwzAMoyTbcdZ0SLsWGLDT/v+rdti52KNt7FjS3EwAIZJ6gIJeMfoYNbF1Lh0pcgDCi0I3Q9JjC/sqzb0b6V+/7y4WBRpiABvTzj8imgWXyHLI+lyvk7yiOohzLkPZroYyLAunU0E5P/S5k1OSvLIlSMNo/WMF33SW4+SAT0eZ9UaD7OXqbwb+JOQfFZblYDkwKfH85SpGcg9PcrmPVL+1rQTQyDQ1hF+3oiCER3TuMLTaxzFuoYAl2trbH/M1TlwEpHQmAAAAAElFTkSuQmCC>

[image6]: <data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAI0AAABPCAIAAABZHNFNAAAR70lEQVR4Xu2ceWxUZRfGp4Ag0G1uWzYtS1hsAhqFiHFJigSNmIKCEQOEKmhYXGowSHEXEwSlimIboQSqKJoosrgEl1bQCrTI2g/aUvYPaFH4w31F+X7feTKT6y1r7LR3yDx/TO68973vvXOe9znnvMudgBNDNCDgLYjBl4jxFB2I8RQdiPEUHYjxFB2I8RQdiPEUHYjxFB2I8RQdiPEUHfA7Tx07duzQoUOrVq1SUlISEhKSkpLS0tLatWvHQTAY5NN7wXkKv/PUqVOnxMTENm3aQEnr1q0DgUD79u0h76KLLmrbti1UeS84awRD8J7wJfzOE9x06dIlNTUVMf31118nTpw4cuRIs2bNkBeFSM17wVkjOQTvCV/C7zxBDxqCmz///BOejh8/fsJQXFwMW5z1XnDWiPH0b5Genn7BBRfg8eLj44cMGfLjjz+KmzBg6++//+YAtSlcOWZ3FIbOvM25gJdLMOA8BwwYkJmZqbtwIU35Odr5kSeyBmkIJv744w+U5OFJ5VD1888///bbb6tXr27RogXWF1XA22IIsAhVCLGsrKyysrJ3796KT127doXsf+NFIw0/8jRz5kw3Jb///rv7qxuQpIPvvvtu2LBhUgYK87YYAtkHZ5cvX75z587c3FwyEdi9+OKLW7ZsyVku917gG/iRp4KCAkz/66+/8qncwQM5PUERC8JGjhxJWhh2g6fCpEmTqqqqSktLlUBS+fDhw82bN4ekzp07e2v7Bn7kCSeGa/r+++9/+eWXE0aVx/VRgsc7YVzCE04MP4nFFZ9wXxzQQniAxdkuBs6Wl5cfPHiQEpJ7uLnpppsOHDhAZbJ/1OZ9FN/AjzxhTdmRDOK4wU0S+OGHH+TxIKy6uhoZ4bgITmQHMEQOUldXR8iBMJiAABq844477rvvvvz8/IqKCtIH+NPAecGCBZ9//jl6cmJ+71wBSVu2bIGG3bt3czxu3DgPT3KJK1eu5CwsQtuyZcuwMgISxxs3bszOzhZDUEjJ3r17IWn9+vXwFBcXp3QfCW7dunXEiBEQjMM8TQLS5PApT+IDR4RKSJ0J9W6e8FQSHMQMHz5cXlF2V16+bdu2F154QaMrCmGCxAHnBn8Ep127du3fv5+Smpoa3GBJSQnVINXPYyk/8iRlIBGc29GjR5WVQUCfPn3gA5eFQYn5opNs8LXXXuMYNcjWHKCeDRs28JWhGFk+okFGtIOSCgsLRRV1OFi7di2qxe8hqdj46dzQq1cvqEJGWBa7wwQJHmMdBIEpsfuoUaMkLOihGpdQUzm3xkA5OTnr1q0bM2YMxzt27EA31KQOFWhBQiSqIa/Nmzc7NtvL2dMnik0LP/JEIoAvmjt3LsYldSbAFBcXw0ptbS0l8nJFRUUca0SMOKZNm8aFcCBfRyEq2bNnD6KBJ1qABhgVQ7CF1Kj2HwN9gqsgiXt5nsQ/8CNPRKPu3btLMVlZWVj5wgsvxPTkC5QcOnSIY3k/xqqU/PTTT1yF4wq3gJMcNGgQJBGEcINK0D1pAqolOElP/ocfecrIyIAJ6HnggQcIUSTffKXX49M46NGjBxaHObwWA6zrrruOQsKVe4VC8YxyMaTJdU/4IXT1799/1apV7kLfwo88MSCFJE24Yes333yTEDVhwgT8oWM+bdGiRXi/Y8eOcSwPRrBxRxcuhBVO4e5OpSfNwPp5bOuGH3lq0aKFkgiMS4bWrVu3YcOGnbBJPMZSmjTq2bMnKlFSwAGUuKMLPhBJKQM8lZ6owFjY7S39DD/ydFIgnby8PBiaPHkyTo/gRBhDQxzwqfThPEbU8IQgkA4MJdguCcdGpvg3aQUx+XmU+u8RNTwRgdob4Il4g4ZwjI4tzCOs+uHnPEPU8KRRDnFL863QNmfOnG+++Wb9+vVk2JDk51nUf4/G40l5M26qefPmN998c79+/cgXiDoaLXlr10PQoGMoWbFixeHDh9977z0GqvPmzTtNiNJVqaH1wyQDZEuCHMhn4lQ1teFPNBJPaQbMgQ4qKiqOHj1aWlpaVVW1Z8+exx57jLzLe0E9hMMP9oXgTZs2jR07FmGJbCTlHj95wKkwMSKJg/nz59fW1pJMKm8kQT9NC02ORuIpxYAhCP7PPvtswIBxt2/fju+CPO8F9SDj0ghh6ZZbbsHE6FJTdrTJeOtUahDBVEMxqQbNqb/44otoUSNoWoita/wfYsixkQ3Glbkvu+yy6urqrVu36tTpkWQ7YZXaTZ06FYJpQfsvYQKnd6pdKJp7ZYwcsNnCoG05QoX5+fkIeunSpdu2bUPiSlK8F/sGjcSTG9iXuLJ3796vv/56xowZ2LpZs2bqy3R5Ig3Jd3p6una/AkjFMdLrZcobbrhh1qxZmJhTcKCNmI7tGaKOZgIDNu1E5KPxzp07f/rpp9Tft28fJHF3Yhtf6R98bt68uaCggHE05dwUUXLtvffem5uby8PIqZaXl4vjoqIiVMhdqNzIpDYBT9gdA5WVlR08eFBbGzCuYxM5eDNSDKxJ4T333LNr1y5MjKFbtWqVbGtOKINLtMq3YcOGKVOmYErOYkSazc7O3r17N2dfeeUVCmFx6NCh/zVg34EDByonvPHGG7/88kuMTpBDZzDKtbSgyQvktcOwZMkSSmgf0WdkZNAyz8OTQ9LZJD4NiybgCeloPQIsXryYX07EohBZzJw5E5GtWbOGyFFYWIjOevfurXxMMUwujpgPx/A6efJkHBeipKm6ujpIhQmO8YrTp09XilhSUsItIFXtpNhGF8rHjx+vFUJKEAdX0TLV8vLy+Dpx4kSO5S1ramqgTa1Jo8qJvD8skmg8npQWY4u4uDgM3dq2smIF3BHccEpLtIQKvmJxzKGFXcck6G4KG5GUowNcE9Voc/jw4chIG5UwLgrAr3JAh0CsWVlZlZWVFOLNArYmQrcYPHiwlkgA3o9r0Rxi4u48Ev2AlhVHN27cyFOhrVTb5u5+kkZDE/CELRYuXNi3b1/sdeutt9LZP/74Y6wQsO0rEMAnZpo9e3bAFiacejxRjt+74oorCGbq2qNHj8aauCMcHYEnMzOTjD9gHnXQoEEB2zqBM6RxPF6KTTVdcsklWuelHe0Uw9F99dVXAVt+hGOkpnVeHphH4glPlVI2ApqAp1WrVuHo+eWEB3yLkmN0gGWJPfRrqj366KO4mm+//fakPGHHI0eOTJo0Sck6fgyz4uuQAiTRbMA2GH344YdYmRLulZOTQza/bNkyLQorSdE2CjoK7lc5CJVXr17do0cPeH3rrbdoCvXjHmFU6WVTzXo0AU/0Sn6wHA7AynKAOB/5GQUSZeEn5YnUTi0EbRG9rUF7JQOWH6rBgG1uwb/RLdQnsPirr77KWS5EK++88w7EEIrEHJfDEI+BvEg9OOAU/QZuaCTZ5iyaar2q8XgKIyUET6FmdDzlZ4TqK8h7z4WggXDAtrJoHKZUImjzFO6anIUVKsN6suE0zTYm/MJT2F7nmkedkSfdC2404nHTU58nTVikGfxDkuMfnoK2lTXF5pbc5WfEGXnSRJGqiRUxUf8ZHHPOGnGHWztptcaHX3gK72441010auc0PGliSWfb2wZ0virJrk8AjpFTQQt7jjUuhXmqNT4iyFOyjUnbhV6jCHsYzebxlRDd2aAtJenp6UGb11EdveUig4bdVNt6706fkadzQlube9TzaOcFSWBT5XhuRJYnrNze9gPL48OZJqe1Xs4pcdarVy/qwJOSPe0WckwKsr62H8leVHPfpWF5kvgYzy1dupRORuI3duzY81xPIDwrCiWXX375yJEjn3vuudzc3DRbZUBPBQUFfGVECROJ9noFRqF+VlbWM888c/XVV2vqT6PObt26Uc0zAdqwPDEyg561a9eWlpbyGIxw8/Pzm3B4G0YEeQradOrOnTsZgVZWVjKe5ZjxCj8exWDuESNGMLJhPMsIt7q6mtGM3Nqll17KJfv27auoqGDQ89RTT3Ehx4x+Wts2ZvddGpYnOlAb23fOkJl+w0B49uzZ5zlP/GbU0NreI5eqwBdffKGZG7ptXV0dw8kHH3wQ00ybNk2vvmBuSIJUdMbBoUOHMBmOCDpLSkpwfXKJYYSHXPJO3FTRBWUk2uqfnGd4cObOFBi0phj4Sn0+uZByRriffPIJCqYbPfLII+GI5VgAC9o7BGk2CMMroHV5dflzefIGRwR5UrwRJfqvjrvvvhtZIJcU2xmJVoYOHcpvxl5Q+MEHH9x2221xcXFbtmxZtGgRJe+//z4642DgwIFVVVU0WH/VJ2hjIPERb2tU9Az6h2znGHOqIIg2rW/JrEpY5GCVGaL4119/nU7DTYlPXEJNJTjiu41tfmprr5NoDyGE4ZbVM/75gA2DCPJEH0ywPVyObSTmN0ASuuHnYZSVK1ciEXoxX6USyMA6/OC5c+euWbNGqgrYih+yGzNmDA2GU4wwNBVL49xr/vz5OFjc47FjxxTzaC3RXopKNPA1YBNL4oNrE22qSbMPxMugrfbihzWzzgP079+fwu7du/P17bff5kkefvjhgC1RLl68mEjG8/ND9u/fL72G09qGRQR5cqz3oQ9yh6KiIpx+WVkZ+sCl0Jc3bdpUWFiYYOvl2j05ZcoUQpFjwTwjI4Mur2V1juEMm2qDQ5Jlj9SXPuJtjziNLF++nDaXLFkSsAlvJ/SK4J133kkd+sfTTz8t0+PWtDzB5RMnTqypqSEOQbDeteaBeUjIwKFBOV8dm6EnXtL+hAkTqNnGtrO/9NJL2w3FxcX3338/d0m2PWv/tEHDIII8afvjtddeixUgCW+Gu4cqnBvlOD2SvZb2/jME0KOxIObWT6U8zWZLYZS41adPHwo5m52djckcE6jiStAGVXy98sorSVIw+tSpU/lKg1yCrenpHGN67rhw4UJ8GpZFml27dqU1ymGCNtEEp1AhnYY73nXXXRRCiV7mRUlaBOF51q1bl2QvWsENPwpJBQyOjaO1ntngiCBPijqaX3j88cf5nXRwrMAn/R3zYS85EMw6btw4ujlnFeHll2CC1Bzz0VUzMzO1qUF2wRFxNt7+2MMJRRds+vLLL+O1cLCt7K/gAvYW24oVKyiEJ+w+fvx4aBg9ejSnIGnOnDncSEERcF+tsIwaNUrk8amlDa4io+EAd9fR/tPgmmuuoUK/fv0S7J0DKSlCg63I8qSMFkNgI34nVlA0BtOnT+dHoi0MMWTIEEzz7rvvUhPO5AwhGAMRaSCGXkwLtbW18+bNo52PPvoIa6JXmKAaHpJ7afVWC0tQjkGffPJJjrkKl4VQuAUlPAD9Q3vTOMC/Ef+JOiSifCJxyukQ+EnuDq+KTPKKXBswp6rsg+E5roI7arFD4g42xPCgPiLIkxNKxvASdFWsEG97j9NCf5mCwtAHPxXFkHkHbLlIOlAcwofAJdaBgwMHDsyaNQt/SFqIRDATGvJ0Xvgg5mE+bkq4kl7hlZEZB7jcVNtWxgiaPIXLySqpA6NaWlywYAFCpAegdYKo8r2HHnooyTa38JzaGXDVVVdR8/rrr+dXoLDBgwdHKBd3ozF4As8//zw8YXE8hhP6TzU67BtvvEHiN2DAAAhAFqn2xlKqLQ/G2wIrTkaJgzaTfPbZZ1CO7aBQTbkxY8YMLEvfx3wc0DjNYlYNfQKWOvbs2ZPPvn37cgsSPM7iWnWKEr2dePvttz/xxBM8LWO7nJwcXVteXg6pqFNLiJorycvL40mSI5PjuRFBnlJcMwXaCIcbhIO29ma50nHHEut2tmdP+RLWTw7NZ+PZHNtNHm+7iJSY6f1qzQ0qbXPfMWCLgQHbGQGohlPiUy2HhwpK0AP2JxTN7H/8cGsBi6bhxfX2NjMJf0k25FIGr2kt6ne0rYZqocMptng2ICLIU6pr4UB/0BC0HIFyERZvc+QiTOVJoddjEkPvAYqJRHuDU4GBduJtMjvJBrbuO2LiTvY/pTgoOOZ2OEzlZnqS9vafpTruYH8765i44212ODm0a0z/L8KzUa41fif0D6hU4GyqvYzVMfT2fCMsxkeQpxQXvOcaDmpcqvWeO48Q4yk6EOMpOhDjKToQQZ5iaEDEeIoOxHiKDsR4ig7EeIoOxHiKDsR4ig7EeIoOxHiKDsR4ig7EeIoO/A/ukEUr9X+NxQAAAABJRU5ErkJggg==>

[image7]: <data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAAAp0lEQVR4XmP4//8/AyUYQ4BUjCFAKmb4z2DMD8RTgfgMED8H4k9A/B8HBsmB1IDUgvTwgwyYgUUhsXgqyICrWCSIxVdABnzBIvH/f0bb///uOZjiqPgTyAB0QQiesOw/GGw/+v+/ZgimPBQTNgAEfvz8/98iAVMNUQYQ4QKKw4DiWJiORYJYDE4HoJQ4DYjPAvGL/4RTIkgNKCWC9PBjpG1SMYYAqRgANW15k6BlxXAAAAAASUVORK5CYII=>

[image8]: <data:image/png;base64,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>

[image9]: <data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAMAAAAoLQ9TAAADAFBMVEX///8AAAAjKC0kKS6XmJoSGSAAAAgrMDUADheGiYsUGyH09fUfJCrq6utCRkqXmZpYXF8AAA9gZGd9gIPOz9BPU1cOFh6OkJLs7OyEhoigoaJTV1owNDkACRPT1NU2Oz9wc3Xf3+AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAADRYK8wAAAAuUlEQVR4XjVO0Y6CMBCc3W2hUhqqSNQHDf7/R/kDdxcuERsU0RZxHmYzszuTFUS4qYSyZn1NImJFmiMK2kUhgNevbAQo41CFaNR4Gd0306PK9DVuHBXx0s5ZpWnPQYGBfjZqKJZywPhY2jHlhgf4zzqiH2+EnLOvBo6cy4a2/n/RJzxLubP8mk+JtX8Di71V/qdddzhLuIenBw7k2ib9QSxmBQKaoneX2dCmm7MtURpE9VKOs0qsXOI3DmsqzoLfhyAAAAAASUVORK5CYII=>

[image10]: <data:image/png;base64,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>