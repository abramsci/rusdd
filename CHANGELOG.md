## [0.0.x] - initial development phase

### [0.0.3] - 2026-04-04
* Field-test for a micro-SD card on Windows platform:
  * `.\target\debug\rusdd.exe --physical-drive \\.\PHYSICALDRIVE3 --output-path output-test.dd`
  * Failed with user-level access == as expected
  * Success with admin-level access
  * Verified result correctness via brief inspection in [HxD editor](https://mh-nexus.de/en/hxd/)
+ Core *input/output* (I/O) skeleton == simplistic version without loop
  + Read a single buffer from a physical drive
  + Write the buffer to a destination filepath

### [0.0.2] - 2026-04-03
+ Printing function with respective HelpLvel enum: Usage, Extended  
+ String constants for respective CLI parts: `ARGUMENTS`, `FLAGS`, `PARAMS`

### [0.0.1] - 2026-04-02
+ Validation and tests for power-of-two size optional parameters
+ *Comand line interface* (CLI) skeleton
  + Required arguments: `--physical-drive`, `--output-path`
  + Logical flags: `--force`, `--smart`
  + Optional parameters: `--sector-size`, `--chunk-size`, `--buffer-size`
+ Description of the tool core intention
