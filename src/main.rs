//! CLI Multifuncional do MOS 6502 / 65C02:
//! Execução de binários, montagem de assembly, depuração interativa REPL,
//! painel visual TUI, desassemblador e emulação do Apple-1 Computer (1976).

use std::env;
use std::fs;
use std::num::ParseIntError;
use std::process;

use mos6502::{
    asm, disasm, graphics::FantasyConsoleBus, systems::basic::BasicSystem, Apple1, Cpu, CpuVariant,
    Debugger, FlatMemory, MemoryBus, TuiDashboard,
};

fn print_banner() {
    println!("\x1b[1;36m");
    println!(r"  __  __  ___  ____    __  ____   ___ ____  ");
    println!(r" |  \/  |/ _ \/ ___|  / /_| ___| / _ \___ \ ");
    println!(r" | |\/| | | | \___ \ | '_ \___ \| | | |__) |");
    println!(r" | |  | | |_| |___) || (_) |__) | |_| / __/ ");
    println!(r" |_|  |_|\___/|____/  \___/____/ \___/_____|");
    println!(
        "\x1b[0m\x1b[33m MOS 6502 / 65C02 Microprocessor Emulator Suite v{}\x1b[0m\n",
        env!("CARGO_PKG_VERSION")
    );
}

fn usage() -> ! {
    print_banner();
    eprintln!(
        "\x1b[1mUSO:\x1b[0m
  mos6502 [OPÇÕES]
  mos6502 <SUBCOMANDO> [ARGUMENTOS]

\x1b[1mSUBCOMANDOS:\x1b[0m
  \x1b[32mapple1\x1b[0m                     Inicia o computador Apple-1 (1976) com o monitor próprio do projeto
  \x1b[32mc64\x1b[0m                        Inicia o computador Commodore 64 com SID 6581 e VIC-II
  \x1b[32mgdb\x1b[0m [--port N]             Inicia servidor GDB Remote Serial Protocol (RSP)
  \x1b[32mbasic\x1b[0m                      Abre o interpretador didático Mini BASIC
  \x1b[32mbench\x1b[0m [--seconds <N>]      Mede o desempenho da CPU por N segundos (padrão: 1)
  \x1b[32mfantasy\x1b[0m <BIN> [--cycles N] Executa um binário na fantasia gráfica 32x32
  \x1b[32masm\x1b[0m <FONTE.asm> [-o <OUT>] Monta arquivo assembly para binário, Intel HEX ou símbolos
  \x1b[32mdisasm\x1b[0m <BIN> [--start <A>] Desmonta arquivo binário em assembly anotado
  \x1b[32manalyze\x1b[0m <BIN> [--base <A>] Analisa CFG estático, sub-rotinas e blocos de código
  \x1b[32mdbg\x1b[0m <BIN|ASM>              Inicia sessão de depuração interativa (REPL estilo GDB)
  \x1b[32mtui\x1b[0m <BIN|ASM>              Inicia o painel visual TUI retrô no terminal
  \x1b[32minfo\x1b[0m                       Exibe estatísticas da CPU, tabela de opcodes e recursos
  \x1b[32mserve\x1b[0m [--port N] [--bin F]  API de depuração em 127.0.0.1 (porta 0 = o SO escolhe)

\x1b[1mOPÇÕES DE EXECUÇÃO:\x1b[0m
  --bin <ARQUIVO>            Carrega e executa imagem binária plana
  --asm <ARQUIVO>            Monta e executa arquivo assembly diretamente
  --variant <nmos|cmos|nes>  Define variante de hardware da CPU (padrão: nmos)
  --load <ENDEREÇO>          Endereço base de carga na memória (padrão: 0x8000)
  -n, --count <N>            Limite de instruções ou ciclos a executar (padrão: 100)
  --mode <insns|cycles>      Modo de contagem de execução (padrão: cycles)
  --trace                    Exibe rastreio linha a linha no formato nestest
  --dump-mem <INÍCIO:TAM>    Despeja bytes de memória após a execução (ex: 0x10:16)
  --tui                      Abre o dashboard TUI com o programa carregado
  --debug                    Abre o debugger interativo com o programa carregado
  -h, --help                 Exibe esta mensagem de ajuda
  -V, --version              Exibe a versão do emulador

\x1b[1mEXEMPLOS:\x1b[0m
  mos6502 apple1
  mos6502 --asm examples/hello_lda.asm.txt -n 20 --trace --dump-mem 0x10:0x01
  mos6502 tui examples/sum_loop.bin
  mos6502 asm program.asm -o program.bin
"
    );
    eprintln!("\n{}", tools::USAGE);
    process::exit(0);
}

fn parse_u16(s: &str) -> Result<u16, ParseIntError> {
    let s = s.trim();
    if let Some(h) = s
        .strip_prefix('$')
        .or_else(|| s.strip_prefix("0x"))
        .or_else(|| s.strip_prefix("0X"))
    {
        u16::from_str_radix(h, 16)
    } else {
        s.parse().or_else(|_| u16::from_str_radix(s, 16))
    }
}

fn parse_address_or_exit(s: &str) -> u16 {
    parse_u16(s).unwrap_or_else(|_| {
        eprintln!("Endereço inválido '{s}': informe decimal, hexadecimal ou $HEX.");
        process::exit(2);
    })
}

fn parse_variant_or_exit(value: &str) -> CpuVariant {
    match value.to_ascii_lowercase().as_str() {
        "nmos" | "6502" => CpuVariant::NMOS,
        "cmos" | "65c02" => CpuVariant::CMOS,
        "nes" | "ricoh" | "2a03" => CpuVariant::Ricoh2A03,
        _ => {
            eprintln!("Variante inválida '{value}': use nmos, cmos ou nes.");
            process::exit(2);
        }
    }
}

/// Erro de uso: mensagem em stderr e saída 2 (como `--variant`).
fn usage_error(message: &str) -> ! {
    eprintln!("{message}");
    process::exit(2);
}

fn value<'a>(args: &'a [String], i: usize, flag: &str) -> &'a str {
    args.get(i)
        .map(String::as_str)
        .unwrap_or_else(|| usage_error(&format!("{flag} exige um valor.")))
}

fn count_or_exit(text: &str, flag: &str) -> u64 {
    text.parse()
        .unwrap_or_else(|_| usage_error(&format!("{flag} exige um inteiro não negativo: {text}")))
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        usage();
    }

    if let Some(code) = tools::dispatch(&args) {
        process::exit(code);
    }

    // Subcomandos dedicados
    match args[0].as_str() {
        "-h" | "--help" | "help" => usage(),
        "-V" | "--version" | "version" => {
            println!(
                "mos6502 v{} (MOS 6502 / 65C02 Emulator Suite)",
                env!("CARGO_PKG_VERSION")
            );
            process::exit(0);
        }
        "c64" => {
            println!("Inicializando Commodore 64...");
            let mut c64 = mos6502::C64Computer::new();
            println!("C64 pronto. Tela inicial (PETSCII 40x25):\n{}", c64.cpu.bus.screen_text());
            process::exit(0);
        }
        "gdb" => {
            let port = args.get(1).and_then(|p| p.parse::<u16>().ok()).unwrap_or(2159);
            let addr = format!("127.0.0.1:{port}");
            let server = match mos6502::GdbServer::bind(&addr) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Erro ao iniciar servidor GDB em {addr}: {e}");
                    process::exit(1);
                }
            };
            println!("Servidor GDB RSP ativo em {addr}. Aguardando cliente (GDB/Ghidra/IDA)...");
            let mem = mos6502::bus::FlatMemory::new();
            let mut cpu = mos6502::Cpu::new(mem);
            if let Err(e) = server.handle_client(&mut cpu, 100_000) {
                eprintln!("Sessão GDB finalizada: {e}");
            }
            process::exit(0);
        }
        "analyze" => {
            if args.len() < 2 {
                eprintln!("Uso: mos6502 analyze <BIN> [--base <ENDEREÇO>]");
                process::exit(2);
            }
            let file_path = &args[1];
            let mut base_addr = 0x8000u16;
            if args.len() >= 4 && args[2] == "--base" {
                base_addr = parse_address_or_exit(&args[3]);
            }
            let bytes = fs::read(file_path).unwrap_or_else(|e| {
                eprintln!("Erro ao ler binário '{file_path}': {e}");
                process::exit(1);
            });
            let rep = mos6502::analyze_binary(&bytes, base_addr);
            println!("\x1b[1;36m=== Relatório de Análise Estática CFG (6502) ===\x1b[0m");
            println!("Arquivo:         {file_path}");
            println!("Endereço base:   ${:04X}", rep.base_addr);
            println!("Tamanho total:   {} bytes", rep.total_bytes);
            println!("Instruções:      {}", rep.instruction_count);
            println!("Sub-rotinas:     {} encontradas", rep.subroutines.len());
            for sub in &rep.subroutines {
                println!("  -> Sub-rotina em ${:04X}", sub);
            }
            println!("Destinos JMP/BR: {} encontrados", rep.jump_targets.len());
            println!("Regiões dados/mortas: {}", rep.unreachable_regions.len());
            for (s, e) in &rep.unreachable_regions {
                println!("  -> [${:04X}..=${:04X}] ({} bytes)", s, e, e.wrapping_sub(*s) + 1);
            }
            process::exit(0);
        }
        "bench" => {
            let target_duration_secs = match args.len() {
                1 => 1.0,
                3 if args[1] == "--seconds" => args[2].parse::<f64>().unwrap_or(f64::NAN),
                _ => f64::NAN,
            };
            if !target_duration_secs.is_finite() || target_duration_secs <= 0.0 {
                eprintln!("Uso: mos6502 bench [--seconds <número finito maior que zero>]");
                process::exit(2);
            }
            match mos6502::bench::run_benchmark(target_duration_secs) {
                Ok(result) => result.print_report(),
                Err(error) => {
                    eprintln!("Erro no benchmark: {error}");
                    process::exit(1);
                }
            }
            process::exit(0);
        }
        "serve" => {
            let mut port: u16 = 0;
            let mut bin_path: Option<String> = None;
            let mut load = 0x8000u16;
            let mut index = 1;
            while index < args.len() {
                match args[index].as_str() {
                    "--port" => {
                        index += 1;
                        port = args
                            .get(index)
                            .and_then(|value| value.parse().ok())
                            .unwrap_or_else(|| {
                                eprintln!("--port exige um inteiro 0..=65535");
                                process::exit(2);
                            });
                    }
                    "--bin" => {
                        index += 1;
                        bin_path = args.get(index).cloned();
                    }
                    "--load" => {
                        index += 1;
                        load = parse_address_or_exit(
                            args.get(index).map(|s| s.as_str()).unwrap_or("0x8000"),
                        );
                    }
                    other => {
                        eprintln!("Opção desconhecida para serve: {other}");
                        process::exit(2);
                    }
                }
                index += 1;
            }
            let mut mem = FlatMemory::new();
            if let Some(path) = bin_path {
                let bytes = fs::read(&path).unwrap_or_else(|error| {
                    eprintln!("Erro ao ler '{path}': {error}");
                    process::exit(1);
                });
                mem.load_at(load, &bytes);
                mem.set_reset_vector(load);
            }
            let mut cpu = Cpu::new(mem);
            cpu.halt_on_brk = true;
            cpu.reset();
            let addr = format!("127.0.0.1:{port}");
            let server = mos6502::debug_api::DebugServer::bind(&addr).unwrap_or_else(|error| {
                eprintln!("Não foi possível escutar em {addr}: {error}");
                process::exit(1);
            });
            let bound = server.local_port().unwrap_or(port);
            println!("mos6502 debug API em http://127.0.0.1:{bound}/");
            println!(
                "GET /registers | POST /step {{\"n\":N}} | GET /memory?start=&len= | PUT /memory"
            );
            if let Err(error) = server.serve_requests(&mut cpu, usize::MAX) {
                eprintln!("Servidor encerrou: {error}");
                process::exit(1);
            }
            process::exit(0);
        }
        "basic" => {
            if let Err(error) = BasicSystem::new().run_interactive() {
                eprintln!("Erro no Mini BASIC: {error}");
                process::exit(1);
            }
            process::exit(0);
        }
        "fantasy" => {
            if args.len() < 2 {
                eprintln!("Uso: mos6502 fantasy <PROGRAMA.bin> [--cycles <N>]");
                process::exit(2);
            }
            let program = fs::read(&args[1]).unwrap_or_else(|error| {
                eprintln!("Erro ao ler '{}': {error}", args[1]);
                process::exit(1);
            });
            let mut cycle_limit = 100_000u64;
            let mut index = 2;
            while index < args.len() {
                if args[index] == "--cycles" {
                    index += 1;
                    cycle_limit = args
                        .get(index)
                        .and_then(|value| value.parse().ok())
                        .unwrap_or_else(|| {
                            eprintln!("--cycles exige um inteiro não negativo.");
                            process::exit(2);
                        });
                } else {
                    eprintln!("Opção desconhecida para fantasy: {}", args[index]);
                    process::exit(2);
                }
                index += 1;
            }

            let mut bus = FantasyConsoleBus::new();
            bus.load_at(0x8000, &program);
            bus.set_reset_vector(0x8000);
            let mut cpu = Cpu::new(bus);
            cpu.halt_on_brk = true;
            cpu.reset();
            if let Err(error) = cpu.run_cycles(cycle_limit) {
                eprintln!("Erro durante a execução: {error}");
                process::exit(1);
            }
            print!("{}", cpu.bus.device.screen.render_to_terminal());
            println!("{}", cpu.registers());
            process::exit(0);
        }
        "apple1" | "--apple1" => {
            let mut apple = Apple1::new();
            if let Err(e) = apple.run_interactive() {
                eprintln!("Erro ao executar Apple-1: {e}");
                process::exit(1);
            }
            process::exit(0);
        }
        "info" => {
            print_banner();
            println!("\x1b[1mEstatísticas do Núcleo MOS 6502:\x1b[0m");
            println!("  * Opcodes mapeados: 256 (151 oficiais + 105 não-documentados/ilegais)");
            println!("  * Modos de endereçamento: 13 (NMOS) + 2 (CMOS: ZpInd, AbsIndX)");
            println!("  * Variantes suportadas: NMOS 6502, CMOS 65C02, Ricoh 2A03 (NES)");
            println!(
                "  * Periféricos modelados: PIA 6820, VIA 6522/LCD, RRIOT 6530 e framebuffer 32x32"
            );
            println!("  * Ambientes: Apple-1 (monitor próprio), Mini BASIC didático, fantasia 6502, modelos parciais KIM-1/Ben Eater");
            println!("  * Ferramentas: Assembler, Intel HEX, símbolos, disassembler, TUI, debugger e benchmark\n");
            process::exit(0);
        }
        "asm" => {
            if args.len() < 2 {
                eprintln!("Uso: mos6502 asm <FONTE.asm> [-o <SAIDA.bin>] [--hex] [--sym]");
                process::exit(1);
            }
            let src_path = &args[1];
            let mut out_path = None;
            let mut emit_hex = false;
            let mut emit_sym = false;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "-o" | "--out" => {
                        i += 1;
                        out_path = Some(value(&args, i, "-o").to_string());
                    }
                    "--hex" => emit_hex = true,
                    "--sym" => emit_sym = true,
                    other => usage_error(&format!("asm: argumento desconhecido: {other}")),
                }
                i += 1;
            }
            let src = fs::read_to_string(src_path).unwrap_or_else(|e| {
                eprintln!("Erro ao ler '{src_path}': {e}");
                process::exit(1);
            });

            match asm::assemble_with_symbols(&src, 0x8000) {
                Ok(output) => {
                    println!("\x1b[32mMontagem concluída com sucesso:\x1b[0m {} bytes gerados (Origem: ${:04X}).", output.bytes.len(), output.origin);
                    if let Some(dest) = out_path {
                        if let Err(e) = fs::write(&dest, &output.bytes) {
                            eprintln!("Erro ao salvar '{dest}': {e}");
                            process::exit(1);
                        }
                        println!("Binário salvo em: {dest}");
                    }
                    if emit_hex {
                        let hex_str = output.to_intel_hex();
                        println!("\nIntel HEX:\n{hex_str}");
                    }
                    if emit_sym {
                        let sym_str = output.to_symbol_table();
                        println!("\nTabela de Símbolos:\n{sym_str}");
                    }
                }
                Err(e) => {
                    eprintln!("\x1b[31mErro de montagem: {e}\x1b[0m");
                    process::exit(1);
                }
            }
            process::exit(0);
        }
        "ines" => {
            let Some(path) = args.get(1) else {
                usage_error("Uso: mos6502 ines <ARQUIVO.nes>");
            };
            if let Some(extra) = args.get(2) {
                usage_error(&format!("ines: argumento desconhecido: {extra}"));
            }
            let data = fs::read(path).unwrap_or_else(|e| {
                eprintln!("Erro ao ler '{path}': {e}");
                process::exit(1);
            });
            match mos6502::ines::Ines::parse(&data) {
                Ok(c) => {
                    println!("formato: {}", if c.nes2 { "NES 2.0" } else { "iNES" });
                    println!("mapper: {} (submapper {})", c.mapper, c.submapper);
                    println!(
                        "PRG: {} bytes, CHR: {} bytes, PRG RAM: {} bytes",
                        c.prg.len(),
                        c.chr.len(),
                        c.prg_ram
                    );
                    println!(
                        "espelhamento: {:?}, bateria: {}, trainer: {}",
                        c.mirroring,
                        c.battery,
                        c.trainer.is_some()
                    );
                    println!("timing: {:?}, sobra: {} bytes", c.timing, c.trailing.len());
                    match c.cartridge() {
                        Ok(_) => println!("mapper suportado"),
                        Err(e) => println!("mapper não suportado: {e}"),
                    }
                }
                Err(e) => {
                    eprintln!("ines: {e}");
                    process::exit(1);
                }
            }
            process::exit(0);
        }
        "disasm" => {
            if args.len() < 2 {
                eprintln!("Uso: mos6502 disasm <ARQUIVO.bin> [--start <ADDR>] [--count <N>]");
                process::exit(1);
            }
            let bin_path = &args[1];
            let mut start = 0x8000u16;
            let mut count = 20usize;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--start" => {
                        i += 1;
                        start = parse_address_or_exit(value(&args, i, "--start"));
                    }
                    "--count" => {
                        i += 1;
                        count = count_or_exit(value(&args, i, "--count"), "--count") as usize;
                    }
                    other => usage_error(&format!("disasm: argumento desconhecido: {other}")),
                }
                i += 1;
            }
            let bytes = fs::read(bin_path).unwrap_or_else(|e| {
                eprintln!("Erro ao ler '{bin_path}': {e}");
                process::exit(1);
            });

            let mut mem = FlatMemory::new();
            mem.load_at(start, &bytes);
            let mut pc = start;
            for _ in 0..count {
                let d = disasm::disasm_at(&mem.data, pc);
                println!("${:04X}:  {:<9}  {}", pc, d.bytes_hex, d.text);
                pc = pc.wrapping_add(d.size as u16);
            }
            process::exit(0);
        }
        _ => {}
    }

    // Processamento de Flags Principais
    let mut bin: Option<String> = None;
    let mut asm_path: Option<String> = None;
    let mut load = 0x8000u16;
    let mut n = 100u64;
    let mut mode_insns = false;
    let mut trace = false;
    let mut dump: Option<(u16, u16)> = None;
    let mut run_tui = false;
    let mut run_debugger = false;
    let mut variant = CpuVariant::NMOS;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "tui" => {
                run_tui = true;
                if let Some(next) = args.get(i + 1) {
                    if !next.starts_with('-') {
                        i += 1;
                        if next.ends_with(".asm") || next.ends_with(".txt") {
                            asm_path = Some(next.clone());
                        } else {
                            bin = Some(next.clone());
                        }
                    }
                }
            }
            "dbg" => {
                run_debugger = true;
                if let Some(next) = args.get(i + 1) {
                    if !next.starts_with('-') {
                        i += 1;
                        if next.ends_with(".asm") || next.ends_with(".txt") {
                            asm_path = Some(next.clone());
                        } else {
                            bin = Some(next.clone());
                        }
                    }
                }
            }
            "--bin" | "-b" => {
                i += 1;
                bin = Some(value(&args, i, "--bin").to_string());
            }
            "--asm" => {
                i += 1;
                asm_path = Some(value(&args, i, "--asm").to_string());
            }
            "--variant" => {
                i += 1;
                let Some(v_str) = args.get(i) else {
                    eprintln!("--variant exige um valor: nmos, cmos ou nes.");
                    process::exit(2);
                };
                variant = parse_variant_or_exit(v_str);
            }
            "--load" => {
                i += 1;
                load = parse_address_or_exit(value(&args, i, "--load"));
            }
            "-n" | "--count" => {
                i += 1;
                n = count_or_exit(value(&args, i, "-n"), "-n");
            }
            "--mode" => {
                i += 1;
                mode_insns = match value(&args, i, "--mode") {
                    "insns" => true,
                    "cycles" => false,
                    other => usage_error(&format!("--mode aceita insns ou cycles: {other}")),
                };
            }
            "--trace" => trace = true,
            "--tui" => run_tui = true,
            "--debug" => run_debugger = true,
            "--dump-mem" => {
                i += 1;
                let spec = value(&args, i, "--dump-mem");
                let Some((a, b)) = spec.split_once(':') else {
                    usage_error(&format!("--dump-mem exige ADDR:LEN: {spec}"));
                };
                dump = Some((parse_address_or_exit(a), parse_address_or_exit(b)));
            }
            other => usage_error(&format!("argumento desconhecido: {other}")),
        }
        i += 1;
    }

    let bytes = if let Some(p) = bin {
        fs::read(&p).unwrap_or_else(|e| {
            eprintln!("Erro ao ler '{p}': {e}");
            process::exit(1);
        })
    } else if let Some(p) = asm_path {
        let src = fs::read_to_string(&p).unwrap_or_else(|e| {
            eprintln!("Erro ao ler '{p}': {e}");
            process::exit(1);
        });
        asm::assemble(&src, load).unwrap_or_else(|e| {
            eprintln!("asm: {e}");
            process::exit(1);
        })
    } else {
        usage();
    };

    let mut mem = FlatMemory::new();
    mem.load_at(load, &bytes);
    mem.set_reset_vector(load);
    mem.set_irq_vector(0);
    mem.set_nmi_vector(0);

    let mut cpu = Cpu::with_variant(mem, variant);
    cpu.halt_on_brk = true;
    cpu.reset();

    if run_tui {
        let mut tui = TuiDashboard::new();
        tui.mem_view_addr = load;
        if let Err(e) = tui.run_loop(&mut cpu) {
            eprintln!("Erro na interface TUI: {e}");
            process::exit(1);
        }
        process::exit(0);
    }

    if run_debugger {
        let mut dbg = Debugger::default();
        if let Err(e) = dbg.interactive_session(&mut cpu) {
            eprintln!("Erro na sessão de depuração: {e}");
            process::exit(1);
        }
        process::exit(0);
    }

    // Execução em linha de comando padrão
    if mode_insns {
        for _ in 0..n {
            if cpu.is_halted() {
                break;
            }
            if trace {
                println!("{}", cpu.trace_line());
            }
            if let Err(error) = cpu.step() {
                eprintln!("Erro durante a execução: {error}");
                process::exit(1);
            }
        }
    } else {
        let start = cpu.registers().cycles;
        while cpu.registers().cycles.saturating_sub(start) < n && !cpu.is_halted() {
            if trace {
                println!("{}", cpu.trace_line());
            }
            if let Err(error) = cpu.step() {
                eprintln!("Erro durante a execução: {error}");
                process::exit(1);
            }
        }
    }

    println!("{}", cpu.registers());
    if let Some((addr, len)) = dump {
        print!("MEM[{addr:04X}]:");
        for offset in 0..len {
            print!(" {:02X}", cpu.bus.peek(addr.wrapping_add(offset)));
        }
        println!();
    }
}

/// Toolchain subcommands with uniform exit codes: 0 success, 1 failure of the input
/// (unreadable file, assembly, link or format error, CPU fault), 2 bad invocation.
/// Errors go to stderr as `mos6502 <cmd>: <message>`.
mod tools {
    use std::collections::HashMap;
    use std::fs;

    use mos6502::coverage::{self, Coverage};
    use mos6502::hexfmt;
    use mos6502::profiler::Profiler;
    use mos6502::snapshot;
    use mos6502::symbols::SymbolTable;
    use mos6502::{asm, audio, disasm, graphics, link, Cpu, CpuVariant, FlatMemory, MemoryBus};

    pub const USAGE: &str = "\
Ferramentas (saída 0 ok, 1 erro na entrada, 2 uso inválido):
  assemble <SRC.asm> -o <OUT> [--format FORMATO] [--sym <ARQ>]
  link <A.asm|A.o65>... -o <OUT.bin> [--base <ADDR>] [--map <ARQ>]
  run <PROG> [--load <ADDR>] [--start <ADDR>] [--variant V] [-n <MAX>]
  trace <PROG> [mesmas opções de run]
  profile <PROG> [opções de run] [--folded <ARQ>] [--dot <ARQ>] [--sym <ARQ>]
  coverage <SRC.asm> [-n <MAX>] [--lcov <ARQ>]
  hex <ENTRADA> -o <SAIDA> [--to FORMATO] [--from FORMATO] [--load <ADDR>]
  pack <ENTRADA> -o <SAIDA> | unpack <ENTRADA> -o <SAIDA>
  nes <ROM.nes> [--frames N] [--region ntsc|pal|dendy] [--ppm F] [--png F]
      [--wav F --rate HZ --bits 8|16 --channels 1|2] [--sav F]
  kim1 [--load F --at ADDR] [--keys \"AD 0200 DA A9\"] [--go ADDR] [-n N]
  board <SRC.asm|PROG> [--cycles N] [--input TEXT] [--dump ADDR:LEN]
  bench-suite [--seconds S] [--baseline F] [--tolerance PCT] [--write F]
Formatos: bin, ihex, srec, mos, prg (C64), xex (Atari); --from força a leitura.
PROG é binário bruto (carregado em --load, padrão $8000) ou Intel HEX, S-record
ou MOS hex (detectados pelo primeiro caractere). A execução para no BRK.";

    struct Failure {
        code: i32,
        message: String,
    }

    fn usage(message: impl Into<String>) -> Failure {
        Failure {
            code: 2,
            message: message.into(),
        }
    }

    fn fail(message: impl Into<String>) -> Failure {
        Failure {
            code: 1,
            message: message.into(),
        }
    }

    type Outcome = Result<(), Failure>;

    /// `Some(exit code)` when `args[0]` names a tool subcommand.
    pub fn dispatch(args: &[String]) -> Option<i32> {
        let cmd = args.first()?.as_str();
        let rest = &args[1..];
        let result = match cmd {
            "assemble" => assemble(rest),
            "link" => link_cmd(rest),
            "run" => run(rest, Mode::Run),
            "trace" => run(rest, Mode::Trace),
            "profile" => run(rest, Mode::Profile),
            "coverage" => coverage_cmd(rest),
            "hex" => hex(rest),
            "nes" => nes(rest),
            "kim1" => kim1(rest),
            "board" => board(rest),
            "bench-suite" => bench_suite(rest),
            "pack" => pack(rest, true),
            "unpack" => pack(rest, false),
            _ => return None,
        };
        Some(match result {
            Ok(()) => 0,
            Err(f) => {
                eprintln!("mos6502 {cmd}: {}", f.message);
                if f.code == 2 {
                    eprintln!("{USAGE}");
                }
                f.code
            }
        })
    }

    /// Flags with a value, flags without, and positionals.
    struct Args {
        values: HashMap<String, String>,
        switches: Vec<String>,
        positional: Vec<String>,
    }

    fn parse(rest: &[String], with_value: &[&str], switches: &[&str]) -> Result<Args, Failure> {
        let mut a = Args {
            values: HashMap::new(),
            switches: Vec::new(),
            positional: Vec::new(),
        };
        let mut i = 0;
        while i < rest.len() {
            let s = rest[i].as_str();
            if with_value.contains(&s) {
                let v = rest
                    .get(i + 1)
                    .ok_or_else(|| usage(format!("{s} exige um valor")))?;
                if a.values.insert(s.to_string(), v.clone()).is_some() {
                    return Err(usage(format!("{s} repetido")));
                }
                i += 2;
            } else if switches.contains(&s) {
                a.switches.push(s.to_string());
                i += 1;
            } else if s.starts_with('-') && s.len() > 1 {
                return Err(usage(format!("opção desconhecida: {s}")));
            } else {
                a.positional.push(s.to_string());
                i += 1;
            }
        }
        Ok(a)
    }

    impl Args {
        fn get(&self, k: &str) -> Option<&str> {
            self.values.get(k).map(String::as_str)
        }
        fn address(&self, k: &str) -> Result<Option<u16>, Failure> {
            self.get(k).map(|v| parse_address(k, v)).transpose()
        }
        fn one_input(&self) -> Result<&str, Failure> {
            match self.positional.as_slice() {
                [one] => Ok(one),
                [] => Err(usage("falta o arquivo de entrada")),
                _ => Err(usage(format!(
                    "argumento inesperado: {}",
                    self.positional[1]
                ))),
            }
        }
        fn output(&self) -> Result<&str, Failure> {
            self.get("-o").ok_or_else(|| usage("falta -o <SAIDA>"))
        }
    }

    fn parse_address(flag: &str, v: &str) -> Result<u16, Failure> {
        let t = v.trim();
        let parsed = if let Some(h) = t.strip_prefix('$') {
            u16::from_str_radix(h, 16)
        } else if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
            u16::from_str_radix(h, 16)
        } else {
            t.parse()
        };
        parsed.map_err(|_| usage(format!("{flag}: endereço inválido: {v}")))
    }

    fn read(path: &str) -> Result<Vec<u8>, Failure> {
        fs::read(path).map_err(|e| fail(format!("não foi possível ler '{path}': {e}")))
    }

    fn read_text(path: &str) -> Result<String, Failure> {
        fs::read_to_string(path).map_err(|e| fail(format!("não foi possível ler '{path}': {e}")))
    }

    fn write(path: &str, bytes: &[u8]) -> Outcome {
        fs::write(path, bytes).map_err(|e| fail(format!("não foi possível gravar '{path}': {e}")))
    }

    const FORMATS: [&str; 6] = ["bin", "ihex", "srec", "mos", "prg", "xex"];

    fn format_named(name: &str) -> Result<&'static str, Failure> {
        let canonical = match name {
            "intel" | "hex" => "ihex",
            "s19" => "srec",
            "kim" => "mos",
            "c64" => "prg",
            "atari" => "xex",
            other => other,
        };
        FORMATS
            .into_iter()
            .find(|f| *f == canonical)
            .ok_or_else(|| usage(format!("formato desconhecido: {name}")))
    }

    fn encode(format: &str, bytes: &[u8], origin: u16, start: u16) -> Result<Vec<u8>, Failure> {
        Ok(match format {
            "ihex" => asm::to_intel_hex(bytes, origin).into_bytes(),
            "srec" => hexfmt::to_srec(bytes, origin, start).into_bytes(),
            "mos" => hexfmt::to_mos_hex(bytes, origin).into_bytes(),
            "prg" => hexfmt::to_prg(bytes, origin).map_err(fail)?,
            "xex" => hexfmt::to_xex(
                &[hexfmt::XexSegment {
                    start: origin,
                    bytes: bytes.to_vec(),
                }],
                Some(start),
            )
            .map_err(fail)?,
            _ => bytes.to_vec(),
        })
    }

    /// Image of a program file. `--from` wins; else `.prg`/`.xex` by extension, an
    /// `$FFFF` header as XEX, text formats by their first character, else raw binary.
    fn load_image(
        path: &str,
        raw: &[u8],
        load: u16,
        from: Option<&str>,
    ) -> Result<hexfmt::HexImage, Failure> {
        let lower = path.to_ascii_lowercase();
        let kind = match from {
            Some(f) => format_named(f)?,
            None if lower.ends_with(".prg") => "prg",
            None if lower.ends_with(".xex") || raw.starts_with(&[0xFF, 0xFF]) => "xex",
            None => "auto",
        };
        match kind {
            "prg" => return hexfmt::from_prg(raw).map_err(fail),
            "xex" => return Ok(hexfmt::from_xex(raw).map_err(fail)?.image()),
            "ihex" | "srec" | "mos" | "auto" => {
                if let Ok(text) = std::str::from_utf8(raw) {
                    if hexfmt::detect_format(text).is_some() {
                        return hexfmt::read_any(text).map_err(|e| fail(e.to_string()));
                    }
                }
                if kind != "auto" {
                    return Err(fail(format!("{path}: não é {kind}")));
                }
            }
            _ => {}
        }
        if usize::from(load) + raw.len() > 0x10000 {
            return Err(fail(format!(
                "{} bytes em ${load:04X} passam de $FFFF",
                raw.len()
            )));
        }
        Ok(hexfmt::HexImage {
            origin: load,
            bytes: raw.to_vec(),
            start: None,
        })
    }

    fn assemble(rest: &[String]) -> Outcome {
        let a = parse(rest, &["-o", "--format", "--sym"], &[])?;
        let input = a.one_input()?;
        let out = a.output()?;
        let format = format_named(a.get("--format").unwrap_or("bin"))?;
        let src = read_text(input)?;
        let o =
            asm::assemble_with_symbols(&src, 0x8000).map_err(|e| fail(format!("{input}: {e}")))?;
        for w in &o.warnings {
            eprintln!("aviso: {w}");
        }
        write(out, &encode(format, &o.bytes, o.origin, o.origin)?)?;
        if let Some(sym) = a.get("--sym") {
            write(sym, o.to_symbol_table().as_bytes())?;
        }
        println!("{} bytes em ${:04X} -> {out}", o.bytes.len(), o.origin);
        Ok(())
    }

    fn link_cmd(rest: &[String]) -> Outcome {
        let a = parse(rest, &["-o", "--base", "--map"], &[])?;
        if a.positional.is_empty() {
            return Err(usage("link exige ao menos um objeto"));
        }
        let out = a.output()?;
        let base = a.address("--base")?.unwrap_or(0x8000);
        let mut objects = Vec::new();
        for path in &a.positional {
            let raw = read(path)?;
            let obj = if path.ends_with(".asm") || path.ends_with(".s") {
                let src =
                    String::from_utf8(raw).map_err(|_| fail(format!("{path}: não é UTF-8")))?;
                link::assemble_object(path, &src)
            } else {
                link::read_object(&raw)
            }
            .map_err(|e| fail(format!("{path}: {e}")))?;
            objects.push(obj);
        }
        let linked = link::link(&objects, base).map_err(|e| fail(e.to_string()))?;
        let low = linked.segments.iter().map(|s| s.address).min();
        let high = linked
            .segments
            .iter()
            .filter(|s| !s.bytes.is_empty())
            .map(|s| s.address.wrapping_add(s.bytes.len() as u16 - 1))
            .max();
        let image = match (low, high) {
            (Some(l), Some(h)) if h >= l => linked.image(l, h, 0xFF),
            _ => Vec::new(),
        };
        write(out, &image)?;
        if let Some(map) = a.get("--map") {
            write(map, linked.map().as_bytes())?;
        }
        println!(
            "{} objetos, {} bytes a partir de ${:04X} -> {out}",
            objects.len(),
            image.len(),
            low.unwrap_or(base)
        );
        Ok(())
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mode {
        Run,
        Trace,
        Profile,
    }

    fn variant(name: &str) -> Result<CpuVariant, Failure> {
        match name {
            "nmos" | "6502" => Ok(CpuVariant::NMOS),
            "cmos" | "65c02" => Ok(CpuVariant::CMOS),
            "nes" | "ricoh" | "2a03" => Ok(CpuVariant::Ricoh2A03),
            other => Err(usage(format!("variante desconhecida: {other}"))),
        }
    }

    fn machine(a: &Args, image: &hexfmt::HexImage) -> Result<Cpu<FlatMemory>, Failure> {
        let mut mem = FlatMemory::new();
        mem.load_at(image.origin, &image.bytes);
        let mut cpu = Cpu::new(mem);
        cpu.variant = variant(a.get("--variant").unwrap_or("nmos"))?;
        cpu.halt_on_brk = true;
        cpu.pc = a
            .address("--start")?
            .or(image.start)
            .unwrap_or(image.origin);
        Ok(cpu)
    }

    fn max_steps(a: &Args) -> Result<u64, Failure> {
        a.get("-n").map_or(Ok(1_000_000), |v| {
            v.parse()
                .map_err(|_| usage(format!("-n exige um inteiro não negativo: {v}")))
        })
    }

    /// Next instruction is `BRK`: the program's end, not executed.
    fn at_brk(cpu: &Cpu<FlatMemory>) -> bool {
        cpu.bus.peek(cpu.pc) == 0x00
    }

    fn registers(cpu: &Cpu<FlatMemory>) -> String {
        format!(
            "PC=${:04X} A=${:02X} X=${:02X} Y=${:02X} SP=${:02X} P=${:02X} ciclos={}",
            cpu.pc,
            cpu.a,
            cpu.x,
            cpu.y,
            cpu.sp,
            cpu.p.raw(),
            cpu.cycles
        )
    }

    fn run(rest: &[String], mode: Mode) -> Outcome {
        let a = parse(
            rest,
            &[
                "--load",
                "--start",
                "--variant",
                "-n",
                "--folded",
                "--dot",
                "--sym",
                "--from",
            ],
            &[],
        )?;
        if mode != Mode::Profile {
            for k in ["--folded", "--dot", "--sym"] {
                if a.get(k).is_some() {
                    return Err(usage(format!("{k} só vale para profile")));
                }
            }
        }
        let input = a.one_input()?;
        let load = a.address("--load")?.unwrap_or(0x8000);
        let image = load_image(input, &read(input)?, load, a.get("--from"))?;
        let mut cpu = machine(&a, &image)?;
        let max = max_steps(&a)?;
        let mut profiler = Profiler::new();
        let mut steps = 0u64;
        while steps < max && !cpu.is_halted() && !at_brk(&cpu) {
            if mode == Mode::Trace {
                let d = disasm::disasm_at_variant(&cpu.bus.data, cpu.pc, cpu.variant);
                println!(
                    "{:04X}  {:<9} {:<14} A:{:02X} X:{:02X} Y:{:02X} P:{:02X} SP:{:02X} CYC:{}",
                    cpu.pc,
                    d.bytes_hex,
                    d.text,
                    cpu.a,
                    cpu.x,
                    cpu.y,
                    cpu.p.raw(),
                    cpu.sp,
                    cpu.cycles
                );
            }
            let r = if mode == Mode::Profile {
                profiler.step(&mut cpu)
            } else {
                cpu.step().map(|_| 0)
            };
            r.map_err(|e| fail(format!("${:04X}: {e}", cpu.pc)))?;
            steps += 1;
        }
        let stopped = cpu.is_halted() || at_brk(&cpu);
        if mode == Mode::Profile {
            let symbols = match a.get("--sym") {
                Some(p) => {
                    Some(SymbolTable::parse(&read_text(p)?).map_err(|e| fail(e.to_string()))?)
                }
                None => None,
            };
            print!("{}", profiler.report(10, symbols.as_ref()));
            if let Some(p) = a.get("--folded") {
                write(p, profiler.folded(symbols.as_ref()).as_bytes())?;
            }
            if let Some(p) = a.get("--dot") {
                write(p, profiler.call_graph_dot().as_bytes())?;
            }
        }
        println!("{steps} instruções; {}", registers(&cpu));
        if !stopped {
            return Err(fail(format!("limite de {max} instruções atingido sem BRK")));
        }
        Ok(())
    }

    /// Every file named by `.include "x"` (recursively), read relative to the
    /// including file's directory, keyed by the name as written.
    fn gather_includes(
        src: &str,
        dir: &std::path::Path,
        files: &mut HashMap<String, String>,
    ) -> Result<(), Failure> {
        for line in src.lines() {
            let t = line.trim_start();
            let Some(rest) = t
                .strip_prefix(".include")
                .or_else(|| t.strip_prefix(".INCLUDE"))
            else {
                continue;
            };
            let Some(name) = rest
                .trim()
                .strip_prefix('"')
                .and_then(|r| r.split('"').next())
            else {
                continue;
            };
            if files.contains_key(name) {
                continue;
            }
            let path = dir.join(name);
            let text = read_text(&path.to_string_lossy())?;
            files.insert(name.to_string(), text.clone());
            gather_includes(&text, path.parent().unwrap_or(dir), files)?;
        }
        Ok(())
    }

    fn coverage_cmd(rest: &[String]) -> Outcome {
        let a = parse(rest, &["-n", "--lcov", "--variant"], &[])?;
        let input = a.one_input()?;
        let src = read_text(input)?;
        let mut files = HashMap::new();
        let dir = std::path::Path::new(input)
            .parent()
            .unwrap_or(std::path::Path::new("."));
        gather_includes(&src, dir, &mut files)?;
        let o = asm::assemble_with_files(&src, 0x8000, &files)
            .map_err(|e| fail(format!("{input}: {e}")))?;
        let image = hexfmt::HexImage {
            origin: o.origin,
            bytes: o.bytes.clone(),
            start: None,
        };
        let mut cpu = machine(&a, &image)?;
        let max = max_steps(&a)?;
        let mut cov = Coverage::new();
        let mut steps = 0;
        while steps < max && !cpu.is_halted() && !at_brk(&cpu) {
            cov.step(&mut cpu)
                .map_err(|e| fail(format!("${:04X}: {e}", cpu.pc)))?;
            steps += 1;
        }
        let map = coverage::line_map(&o, input);
        let total: usize = map.iter().map(|f| f.lines.len()).sum();
        let hit: usize = map
            .iter()
            .map(|f| {
                f.lines
                    .iter()
                    .filter(|(_, ad)| cov.line_hits(ad) > 0)
                    .count()
            })
            .sum();
        println!("{hit}/{total} linhas executadas em {steps} instruções");
        for f in &map {
            for (line, addrs) in &f.lines {
                if cov.line_hits(addrs) == 0 {
                    println!("  não executada: {}:{line}", f.file);
                }
            }
        }
        if let Some(p) = a.get("--lcov") {
            write(p, cov.to_lcov_files("mos6502", &map).as_bytes())?;
        }
        Ok(())
    }

    fn hex(rest: &[String]) -> Outcome {
        let a = parse(rest, &["-o", "--to", "--load", "--from"], &[])?;
        let input = a.one_input()?;
        let out = a.output()?;
        let to = format_named(a.get("--to").unwrap_or("bin"))?;
        let load = a.address("--load")?.unwrap_or(0x8000);
        let image = load_image(input, &read(input)?, load, a.get("--from"))?;
        let start = image.start.unwrap_or(image.origin);
        write(out, &encode(to, &image.bytes, image.origin, start)?)?;
        println!(
            "{} bytes em ${:04X} -> {out}",
            image.bytes.len(),
            image.origin
        );
        Ok(())
    }

    fn pack(rest: &[String], compress: bool) -> Outcome {
        let a = parse(rest, &["-o"], &[])?;
        let input = a.one_input()?;
        let out = a.output()?;
        let raw = read(input)?;
        let result = if compress {
            snapshot::pack(&raw)
        } else {
            snapshot::unpack(&raw)
        }
        .map_err(|e| fail(format!("{input}: {e}")))?;
        write(out, &result)?;
        println!("{} -> {} bytes", raw.len(), result.len());
        Ok(())
    }

    fn number<T: std::str::FromStr>(a: &Args, flag: &str, default: T) -> Result<T, Failure> {
        a.get(flag).map_or(Ok(default), |v| {
            v.parse()
                .map_err(|_| usage(format!("{flag}: número inválido: {v}")))
        })
    }

    fn nes(rest: &[String]) -> Outcome {
        use mos6502::ppu::Region;
        use mos6502::systems::nes2a03::Nes2A03;
        let a = parse(
            rest,
            &[
                "--frames",
                "--region",
                "--ppm",
                "--png",
                "--wav",
                "--rate",
                "--bits",
                "--channels",
                "--sav",
            ],
            &[],
        )?;
        let input = a.one_input()?;
        let frames: u64 = number(&a, "--frames", 60)?;
        let (region, cpu_hz) = match a.get("--region").unwrap_or("ntsc") {
            "ntsc" => (Region::Ntsc, 1_789_773.0),
            "pal" => (Region::Pal, 1_662_607.0),
            "dendy" => (Region::Dendy, 1_773_448.0),
            other => return Err(usage(format!("região desconhecida: {other}"))),
        };
        let format = audio::WavFormat {
            sample_rate: number(&a, "--rate", 44_100)?,
            bits: number(&a, "--bits", 16)?,
            channels: number(&a, "--channels", 1)?,
        };
        if a.get("--wav").is_some() {
            audio::wav_bytes_with(&[], format).map_err(usage)?;
        }
        let rom = read(input)?;
        let mut nes = Nes2A03::from_ines(&rom).map_err(|e| fail(format!("{input}: {e}")))?;
        nes.set_region(region);
        if let Some(sav) = a.get("--sav") {
            let loaded = nes
                .read_sav(std::path::Path::new(sav))
                .map_err(|e| fail(format!("{sav}: {e}")))?;
            if loaded {
                println!("bateria restaurada de {sav}");
            }
        }
        let mut samples = Vec::new();
        if a.get("--wav").is_some() {
            let per_sample = cpu_hz / f64::from(format.sample_rate);
            let target = nes.cpu.bus.ppu.frame + frames;
            let mut owed = 0.0f64;
            while nes.cpu.bus.ppu.frame < target {
                owed += per_sample;
                let n = owed.floor();
                owed -= n;
                nes.run_cycles(n as u64).map_err(fail)?;
                samples.push(nes.audio_sample());
            }
        } else {
            nes.run_frames(frames).map_err(fail)?;
        }
        println!(
            "{frames} quadros, {} ciclos de CPU, PC=${:04X}",
            nes.cpu.cycles, nes.cpu.pc
        );
        let rgb = nes.cpu.bus.ppu.rgb_frame();
        let (w, h) = (mos6502::ppu::WIDTH as u32, mos6502::ppu::HEIGHT as u32);
        if let Some(p) = a.get("--ppm") {
            write(p, &graphics::ppm_rgb(w, h, &rgb))?;
        }
        if let Some(p) = a.get("--png") {
            write(p, &graphics::png_rgb(w, h, &rgb, 6).map_err(fail)?)?;
        }
        if let Some(p) = a.get("--wav") {
            let mono = audio::pcm_from_levels(&samples);
            let pcm = if format.channels == 2 {
                audio::interleave(&mono, &mono).map_err(fail)?
            } else {
                mono
            };
            write(p, &audio::wav_bytes_with(&pcm, format).map_err(fail)?)?;
            println!("{} amostras -> {p}", samples.len());
        }
        if let Some(sav) = a.get("--sav") {
            match nes.write_sav(std::path::Path::new(sav)) {
                Ok(true) => println!("bateria salva em {sav}"),
                Ok(false) => println!("cartucho sem bateria: {sav} não gravado"),
                Err(e) => return Err(fail(format!("{sav}: {e}"))),
            }
        }
        Ok(())
    }

    fn kim1(rest: &[String]) -> Outcome {
        use mos6502::systems::kim1::{Key, Kim1};
        let a = parse(rest, &["--load", "--at", "--keys", "--go", "-n"], &[])?;
        if !a.positional.is_empty() {
            return Err(usage(format!("argumento inesperado: {}", a.positional[0])));
        }
        let mut kim = Kim1::new();
        kim.run_instructions(2000).map_err(fail)?;
        if let Some(path) = a.get("--load") {
            let at = a.address("--at")?.unwrap_or(0x0200);
            let image = load_image(path, &read(path)?, at, None)?;
            for (i, &b) in image.bytes.iter().enumerate() {
                let addr = image.origin.wrapping_add(i as u16);
                kim.cpu.bus.write(addr, b);
                if kim.cpu.bus.peek(addr) != b {
                    return Err(fail(format!("${addr:04X} não é RAM gravável do KIM-1")));
                }
            }
            println!(
                "{} bytes carregados em ${:04X}",
                image.bytes.len(),
                image.origin
            );
        } else if a.get("--at").is_some() {
            return Err(usage("--at exige --load"));
        }
        if let Some(keys) = a.get("--keys") {
            for token in keys.split_whitespace() {
                match token.to_ascii_uppercase().as_str() {
                    "AD" => kim.tap(Key::Ad),
                    "DA" => kim.tap(Key::Da),
                    "+" => kim.tap(Key::Plus),
                    "GO" => kim.tap(Key::Go),
                    "PC" => kim.tap(Key::Pc),
                    "RS" => {
                        kim.rs();
                        kim.run_instructions(2000).map(|_| ())
                    }
                    "ST" => {
                        kim.st();
                        kim.run_instructions(2000).map(|_| ())
                    }
                    digits => {
                        for c in digits.chars() {
                            let d = c
                                .to_digit(16)
                                .ok_or_else(|| usage(format!("tecla desconhecida: {token}")))?;
                            kim.tap(Key::Hex(d as u8)).map_err(fail)?;
                        }
                        Ok(())
                    }
                }
                .map_err(fail)?;
            }
        }
        if let Some(go) = a.address("--go")? {
            kim.cpu.pc = go;
        }
        let n: u64 = number(&a, "-n", 20_000)?;
        kim.run_instructions(n).map_err(fail)?;
        println!("display: [{}]", kim.cpu.bus.display_text());
        println!(
            "PC=${:04X} A=${:02X} X=${:02X} Y=${:02X}",
            kim.cpu.pc, kim.cpu.a, kim.cpu.x, kim.cpu.y
        );
        Ok(())
    }

    /// RAM with a 6522 at `$C000-$C00F` and a 6551 at `$D000-$D003`, the layout the
    /// `examples/via_*.asm` and `acia_*.asm` programs expect.
    struct Board {
        ram: FlatMemory,
        via: mos6502::via::Via6522,
        acia: mos6502::acia::Acia6551,
    }

    impl MemoryBus for Board {
        fn read(&mut self, addr: u16) -> u8 {
            match addr {
                0xC000..=0xC00F => self.via.read((addr & 0x0F) as u8),
                0xD000..=0xD003 => mos6502::bus::IoDevice::read(&mut self.acia, addr & 3),
                _ => self.ram.read(addr),
            }
        }
        fn write(&mut self, addr: u16, value: u8) {
            match addr {
                0xC000..=0xC00F => self.via.write((addr & 0x0F) as u8, value),
                0xD000..=0xD003 => mos6502::bus::IoDevice::write(&mut self.acia, addr & 3, value),
                _ => self.ram.write(addr, value),
            }
        }
        fn peek(&self, addr: u16) -> u8 {
            self.ram.peek(addr)
        }
    }

    fn parse_dump(text: &str) -> Result<(u16, u16), Failure> {
        let (s, l) = text
            .split_once(':')
            .ok_or_else(|| usage(format!("--dump espera ADDR:LEN: {text}")))?;
        Ok((parse_address("--dump", s)?, parse_address("--dump", l)?))
    }

    fn board(rest: &[String]) -> Outcome {
        let a = parse(
            rest,
            &[
                "--cycles", "--input", "--dump", "--load", "--reset", "--irq", "--from",
            ],
            &[],
        )?;
        let input = a.one_input()?;
        let cycles: u64 = number(&a, "--cycles", 100_000)?;
        let dump = a.get("--dump").map(parse_dump).transpose()?;
        let image = if input.ends_with(".asm") || input.ends_with(".s") {
            let src = read_text(input)?;
            let o = asm::assemble_with_symbols(&src, 0x8000)
                .map_err(|e| fail(format!("{input}: {e}")))?;
            hexfmt::HexImage {
                origin: o.origin,
                bytes: o.bytes,
                start: None,
            }
        } else {
            let load = a.address("--load")?.unwrap_or(0x8000);
            load_image(input, &read(input)?, load, a.get("--from"))?
        };
        let mut ram = FlatMemory::new();
        ram.load_at(image.origin, &image.bytes);
        let reset = a
            .address("--reset")?
            .or(image.start)
            .unwrap_or(image.origin);
        let irq = a.address("--irq")?.unwrap_or(image.origin.wrapping_add(3));
        ram.set_reset_vector(reset);
        ram.set_irq_vector(irq);
        let mut cpu = Cpu::new(Board {
            ram,
            via: mos6502::via::Via6522::new(),
            acia: mos6502::acia::Acia6551::new(),
        });
        cpu.reset();
        let mut pending: std::collections::VecDeque<u8> =
            a.get("--input").unwrap_or("").bytes().collect();
        let mut tx = Vec::new();
        let mut done = 0u64;
        let mut next_feed = 200u64;
        while done < cycles {
            if done >= next_feed {
                if let Some(b) = pending.pop_front() {
                    cpu.bus.acia.push_rx(b);
                }
                next_feed += 200;
            }
            let used = u64::from(
                cpu.step()
                    .map_err(|e| fail(format!("${:04X}: {e}", cpu.pc)))?,
            );
            for _ in 0..used {
                cpu.bus.via.tick();
                cpu.bus.acia.tick();
                if let Some(b) = cpu.bus.acia.pop_tx() {
                    tx.push(b);
                }
            }
            done += used;
            if cpu.bus.via.irq_asserted() || cpu.bus.acia.irq_asserted() {
                cpu.irq();
            }
        }
        println!(
            "{done} ciclos; PC=${:04X} A=${:02X} X=${:02X} Y=${:02X}",
            cpu.pc, cpu.a, cpu.x, cpu.y
        );
        println!(
            "VIA: ORB=${:02X} PB=${:02X}",
            cpu.bus.via.orb_latch(),
            cpu.bus.via.port_b_pins()
        );
        println!("ACIA tx: {:?}", String::from_utf8_lossy(&tx));
        if let Some((start, len)) = dump {
            for row in (0..len).step_by(16) {
                let addr = start.wrapping_add(row);
                let n = (len - row).min(16);
                let bytes: Vec<String> = (0..n)
                    .map(|i| format!("{:02X}", cpu.bus.ram.peek(addr.wrapping_add(i))))
                    .collect();
                println!("{addr:04X}: {}", bytes.join(" "));
            }
        }
        if !pending.is_empty() {
            return Err(fail(format!(
                "{} bytes de --input não couberam em {cycles} ciclos",
                pending.len()
            )));
        }
        Ok(())
    }

    fn bench_suite(rest: &[String]) -> Outcome {
        use mos6502::bench::{run_suite, SuiteResult};
        let a = parse(
            rest,
            &["--seconds", "--baseline", "--tolerance", "--write"],
            &[],
        )?;
        if !a.positional.is_empty() {
            return Err(usage(format!("argumento inesperado: {}", a.positional[0])));
        }
        let seconds: f64 = number(&a, "--seconds", 0.5)?;
        let tolerance: f64 = number(&a, "--tolerance", 15.0)?;
        if !(seconds.is_finite() && seconds > 0.0 && tolerance.is_finite() && tolerance >= 0.0) {
            return Err(usage("--seconds > 0 e --tolerance >= 0"));
        }
        let baseline = match a.get("--baseline") {
            Some(p) => Some(
                SuiteResult::parse_baseline(&read_text(p)?)
                    .map_err(|e| fail(format!("{p}: {e}")))?,
            ),
            None => None,
        };
        let result = run_suite(seconds).map_err(fail)?;
        print!("{}", result.report());
        if let Some(p) = a.get("--write") {
            write(p, result.to_baseline().as_bytes())?;
        }
        if let Some(b) = baseline {
            let regressions = result.compare(&b, tolerance);
            if !regressions.is_empty() {
                for r in &regressions {
                    eprintln!("regressão: {r}");
                }
                return Err(fail(format!(
                    "{} regressões (tolerância {tolerance}%)",
                    regressions.len()
                )));
            }
            println!("sem regressões (tolerância {tolerance}%)");
        }
        Ok(())
    }
}
