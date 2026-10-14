# focus-timer

Temporizador Pomodoro para o terminal, escrito em Rust. Escolha entre um relógio com ponteiros animados e uma ampulheta que transfere a areia ao longo da sessão.

## Instalação

```sh
cargo install --path .
```

Após a publicação no crates.io, instale com `cargo install focus-timer-cli`. O comando instalado se chama `focus-timer`.

## Uso

```sh
focus-timer --mode focus --view clock --minutes 25
focus-timer --mode focus --view hourglass --minutes 25
focus-timer --mode break --view hourglass --minutes 5
```

Os padrões são `--mode focus`, `--view clock` e 25 minutos para foco ou 5 minutos para pausa. `--minutes` aceita valores de 1 a 1440.

Durante a sessão, pressione `espaço` para pausar ou continuar, `r` para reiniciar e `q` ou `Ctrl+C` para sair. Após o término, `r` inicia outra sessão. Para ver todas as opções, execute `focus-timer --help`.

O desenho se adapta ao tamanho da janela. Em terminais muito pequenos, o tempo e os controles continuam visíveis em uma versão resumida.
