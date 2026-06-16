package work.slhaf.agentic.console

interface Platform {
    val name: String
}

expect fun getPlatform(): Platform