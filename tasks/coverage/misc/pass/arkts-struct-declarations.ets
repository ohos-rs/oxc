interface First {}
interface Second {}
class Base {}

export abstract struct Derived<T> extends Base implements First, Second {
  static {
    initialize()
  }
  static readonly values: string[]
  readonly [key: string]: unknown
  accessor current: T
  constructor(value: T) {
    this.current = value
  }
  abstract render(): void
  get value(): T {
    return this.current
  }
  set value(next: T) {
    this.current = next
  }
}

