# focus-timer

Temporizador Pomodoro para o terminal, escrito em Rust, com uma ampulheta animada que transfere a areia ao longo da sessão.

## Instalação

Com Rust e Cargo instalados, instale o pacote publicado no [crates.io](https://crates.io/crates/focus-timer-cli):

```sh
cargo install focus-timer-cli
```

O comando instalado se chama `focus-timer`. Para instalar a partir deste repositório, execute `cargo install --path .` na pasta do projeto.

## Uso

```sh
focus-timer
focus-timer --mode focus --minutes 25
focus-timer --mode break --minutes 5
```

O padrão é iniciar com a ampulheta em uma sessão de foco de 25 minutos. O ciclo avança automaticamente: foco de 25 minutos, pausa curta de 5 minutos e novo foco. Depois de cada quatro sessões de foco, a pausa dura 15 minutos; o ciclo então recomeça. `--minutes` aceita valores de 1 a 1440 e altera a duração do modo inicial. Se o modo inicial for foco, essa duração também vale para os focos seguintes; ao iniciar em pausa, os focos seguintes duram 25 minutos.

Durante a sessão, pressione `espaço` para pausar ou continuar, `r` para reiniciar a etapa atual e `q` ou `Ctrl+C` para sair. Para ver todas as opções, execute `focus-timer --help`.

O desenho se adapta ao tamanho da janela. Em terminais muito pequenos, o tempo e os controles continuam visíveis em uma versão resumida.

## Licença

MIT. Consulte o arquivo [LICENSE](LICENSE).
