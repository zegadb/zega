a simple way to start docker containers and watch for updates to the source image

think of it as an easy way to interface with `docker` & `cron`


## install

`npm install -g kebele`

*the alias* `kb` *will be added if available on your system*


## usage
`kebele config` - configure runtime settings

`kebele add` - setup a new container

`kebele status` - list all docker containers


## features

- [x] Start new container with user provided settings
- [x] Get currently running docker containers
- [ ] Get currently running cron jobs
- [ ] Create cron job based on user schedule
- [ ] Display summary of commands that were run for docker + cron during init
- [ ] Send http webhook when container upgrade done
- [ ] Modify running cron jobs


## os support

- [x] Linux
- [ ] macOS
- [ ] Windows


## security

communication with docker is done over its unix socket, **you do not need to enable http on port 2375**.

docker API poses security concerns if your server is exposed to public internet without security protections in place.

the `config` wizard will do what it can to detect environment settings and suggest changes.

once kebele nears **v1.0**, work will be done to automate the process of creating a CA and installing certificates. this is so your server that is running the docker containers adheres to the [security best practices](https://docs.docker.com/engine/security/protect-access/) recommended by the docker team. there will also be testing done to ensure kebele works if the server is setup to [run docker as a non-root user](https://docs.docker.com/engine/install/linux-postinstall/#manage-docker-as-a-non-root-user).

if you have any recommendations for security best practices that can be incorporated into this cli tool, [please submit an issue](https://github.com/samifouad/kebele/issues).


## contribute

contributions are welcome! please keep in mind this is going to remain extremely simple to avoid tech debt and to ensure maintinence is easy long into the future.
