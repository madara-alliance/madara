


#### Task Need To Be Done

- [ ] Added the worker support
- [X] Added the penning setup support
- [ ] Added Documentation for the application
- [ ] Adding Proper Logging to the application
- [ ] Adding a proper tracing to the application

- [ ] Support for the Session layer in the application which make application more atomic
- [ ] Introduction to Anyhow in the application instead of the eyre [make's the debugging easier]

 


### Future Scope

- Crate Isolation based on execution
  - Right now all the code are inside single crate and started in same pod / container
  - But it is needed not to be in same load since some pod might have external dependencies and other might have Resource depdendecy 
  - It is always recommended to have different crate for different execution
    - Api Server
    - Core Lib
    - Queue Consumer
    - Setup [since this can be moved to a separate codebase in future]