@Component
struct UICallbacks {
  build() {
    ForEach(items, (item: string) => {
      Text(item)
    })
    LazyForEach(source, (item: string) => {
      Row() {
        Text(item)
      }
    })
    Repeat(items).each((item: string) => {
      Text(item)
    })
  }
}

